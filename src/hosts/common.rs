#![forbid(unsafe_code)]

//! Routes and helpers every surface shares.
//!
//! Health, logout, the WebSocket relay, the chat proxy and the on-intent
//! prefetch endpoint exist identically on `app.`, `user.`, `org.` and `m.`, so
//! they are defined once here and merged into each surface's router.
//!
//! [`render`] and [`error_page`] are the only two ways a handler in this crate
//! turns markup into a response — which is what guarantees every page goes
//! through the shell, and therefore through the CSP nonce and the CSRF token.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Extension, Form, Router};
use maud::Markup;
use serde::Deserialize;

use crate::chat::{audience_from_param, ChatSession};
use crate::error::WebError;
use crate::middleware::RequestCtx;
use crate::session::with_cookie;
use crate::state::AppState;
use crate::ui::layout::{page, PageMeta};
use crate::ui::{chat_widget, components, loader};

/// Wraps markup in the shell, or returns it bare when htmx asked for a fragment.
#[must_use]
pub fn render(context: &RequestCtx, meta: &PageMeta, body: Markup) -> Response {
    if context.is_htmx {
        return body.into_response();
    }
    page(&context.page(), meta, body).into_response()
}

/// A full error page on the same shell, so a 404 still looks like the product.
#[must_use]
pub fn error_page(context: &RequestCtx, error: &WebError) -> Response {
    let body = components::section("", error.headline(), components::error_block(error));
    let meta = PageMeta::new(error.headline(), error.detail());
    let mut response = render(context, &meta, body);
    *response.status_mut() = error.status();
    response
}

/// Renders `result`, or an error page.
#[must_use]
pub fn render_result(context: &RequestCtx, meta: &PageMeta, result: Result<Markup, WebError>) -> Response {
    match result {
        Ok(body) => render(context, meta, body),
        Err(error) => error_page(context, &error),
    }
}

/// The routes every surface mounts.
#[must_use]
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/ws", get(crate::ws::handler))
        .route("/logout", post(logout))
        .route("/chat/session", get(chat_open))
        .route("/chat/messages", get(chat_history).post(chat_send))
        .route(loader::PREFETCH_PATH, get(prefetch_hints))
}

/// Liveness. Deliberately trivial: it must not depend on the database or the
/// api-server, or a dependency outage would take the fleet's health signal down
/// with it.
async fn healthz() -> Response {
    (
        StatusCode::OK,
        [("content-type", "application/json")],
        r#"{"status":"ok","service":"gha-indie-worker-web-server"}"#,
    )
        .into_response()
}

/// Readiness. Reports which read avenue is live without asserting on it.
async fn readyz(State(state): State<AppState>) -> Response {
    let body = serde_json::json!({
        "status": "ok",
        "service": crate::SERVICE_NAME,
        "readSource": state.repo.read_source().as_str(),
        "sharedAuthConfigured": state.auth.is_configured(),
        "chatEnabled": state.chat.is_enabled(),
    });
    (StatusCode::OK, axum::Json(body)).into_response()
}

#[derive(Debug, Deserialize)]
pub struct CsrfForm {
    #[serde(default)]
    pub csrf_token: Option<String>,
}

/// Clears the host-scoped session cookie and returns to the surface's home.
async fn logout(
    Extension(context): Extension<RequestCtx>,
    State(state): State<AppState>,
    Form(form): Form<CsrfForm>,
) -> Response {
    if let Err(error) = context.verify_form_csrf(&state.csrf, form.csrf_token.as_deref()) {
        return error_page(&context, &error);
    }
    with_cookie(Redirect::to("/").into_response(), state.sessions.clear_cookie())
}

#[derive(Debug, Deserialize)]
pub struct ChatQuery {
    #[serde(default)]
    pub surface: Option<String>,
    #[serde(default)]
    pub session: Option<String>,
}

/// Opens a chat session and returns the log fragment htmx swaps in.
async fn chat_open(
    Extension(context): Extension<RequestCtx>,
    State(state): State<AppState>,
    Query(query): Query<ChatQuery>,
) -> Response {
    let audience = audience_from_param(query.surface.as_deref());
    match state.chat.open(audience, context.bearer(), &context.path).await {
        Ok(session) => chat_fragment(&session).into_response(),
        Err(_) => components::alert(
            components::Tone::Warn,
            "Chat is not available",
            "Try again shortly, or email support@indiebuild.dev.",
        )
        .into_response(),
    }
}

async fn chat_history(
    Extension(context): Extension<RequestCtx>,
    State(state): State<AppState>,
    Query(query): Query<ChatQuery>,
) -> Response {
    let audience = audience_from_param(query.surface.as_deref());
    let Some(session_id) = query.session.as_deref().filter(|id| crate::ws::is_safe_id(id)) else {
        return error_page(&context, &WebError::BadRequest("That chat session is not valid."));
    };
    match state.chat.history(audience, context.bearer(), session_id).await {
        Ok(session) => chat_fragment(&session).into_response(),
        Err(error) => error.into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct ChatMessageForm {
    #[serde(default)]
    pub csrf_token: Option<String>,
    #[serde(default)]
    pub surface: Option<String>,
    #[serde(default)]
    pub session: Option<String>,
    #[serde(default)]
    pub message: String,
}

async fn chat_send(
    Extension(context): Extension<RequestCtx>,
    State(state): State<AppState>,
    Form(form): Form<ChatMessageForm>,
) -> Response {
    if let Err(error) = context.verify_form_csrf(&state.csrf, form.csrf_token.as_deref()) {
        return error.into_response();
    }
    let audience = audience_from_param(form.surface.as_deref());
    let session_id = match form.session.as_deref().filter(|id| crate::ws::is_safe_id(id)) {
        Some(id) => id.to_owned(),
        None => match state.chat.open(audience, context.bearer(), &context.path).await {
            Ok(session) => session.id,
            Err(error) => return error.into_response(),
        },
    };
    match state
        .chat
        .send(audience, context.bearer(), &session_id, &form.message)
        .await
    {
        Ok(session) => chat_fragment(&session).into_response(),
        Err(error) => error.into_response(),
    }
}

fn chat_fragment(session: &ChatSession) -> Markup {
    if session.streaming {
        chat_widget::live_log(&session.id, &session.rendered())
    } else {
        chat_widget::message_list(&session.rendered())
    }
}

/// The release prefetch hints, served only when the visitor showed intent.
async fn prefetch_hints(Extension(context): Extension<RequestCtx>) -> Response {
    loader::release_prefetch_hint(&context.page()).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hosts::Surface;

    fn context(is_htmx: bool) -> RequestCtx {
        RequestCtx {
            surface: Surface::App,
            host: "app.indiebuild.dev".into(),
            base_domain: "indiebuild.dev".into(),
            path: "/".into(),
            nonce: "n0nce".into(),
            csrf_token: "tok3n".into(),
            csrf: crate::csrf::CsrfCheck::NotRequired,
            actor: None,
            is_htmx,
            subject: crate::csrf::ANONYMOUS_SUBJECT.into(),
            release_manifest_url: None,
            chat_enabled: true,
        }
    }

    #[test]
    fn an_htmx_request_gets_a_fragment_and_a_browser_gets_the_shell() {
        let body = maud::html! { p { "fragment" } };
        let fragment = render(&context(true), &PageMeta::new("t", "d"), body.clone());
        assert_eq!(fragment.status(), StatusCode::OK);

        let full = render(&context(false), &PageMeta::new("t", "d"), body);
        assert_eq!(full.status(), StatusCode::OK);
    }

    #[test]
    fn an_error_page_keeps_the_error_status() {
        let response = error_page(&context(false), &WebError::NotFound);
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let forbidden = error_page(&context(false), &WebError::Forbidden);
        assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn render_result_reports_the_error_rather_than_an_empty_page() {
        let response = render_result(&context(false), &PageMeta::new("t", "d"), Err(WebError::Unavailable));
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}
