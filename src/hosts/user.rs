#![forbid(unsafe_code)]

//! `user.indiebuild.dev` — the B2C surface.
//!
//! Personal accounts: sign up, sign in, verify an address, a personal workspace,
//! API tokens, and the step-up factors (TOTP and passkeys) that shared-auth
//! actually offers.
//!
//! Credentials never touch this server. Sign-up and sign-in hand off to
//! shared-auth and come back through `/auth/callback`, where
//! [`crate::session::complete_exchange`] mints this origin's cookie. TOTP
//! enrolment and confirmation call shared-auth directly with the session's
//! delegated access token, and the resulting `acr` is written back into the
//! cookie so a stepped-up session stays stepped up.
//!
//! The passkey ceremony needs WebAuthn, which is browser API work: the server
//! renders the ceremony options into a mount that the `islands` bundle drives,
//! and takes the finished credential back on `/security/passkey/finish`. With
//! islands compiled out, the page says passkeys need the app bundle rather than
//! offering a button that cannot work.

use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Extension, Form, Router};
use maud::{html, Markup};
use serde::Deserialize;
use shared_auth_client::{CeremonyStart, Factor, TotpEnrollment};

use crate::data::models::TokenSummary;
use crate::error::WebError;
use crate::hosts::{common, Surface};
use crate::middleware::RequestCtx;
use crate::state::AppState;
use crate::ui::chat_widget::ChatAudience;
use crate::ui::layout::PageMeta;
use crate::ui::{components, forms, loader, tables};

/// One scope a personal API token may carry.
///
/// `field` is the form field name. HTML checkboxes post repeated keys, which
/// `serde_urlencoded` (what `axum::Form` uses) cannot collect into a `Vec`, so
/// each scope gets its own field and the set is rebuilt from the declared list.
/// That also means an unknown scope in a hand-crafted POST is simply ignored.
#[derive(Clone, Copy, Debug)]
pub struct TokenScope {
    pub value: &'static str,
    pub field: &'static str,
    pub label: &'static str,
}

/// Scopes a personal API token may be given. Anything else is refused.
pub const TOKEN_SCOPES: &[TokenScope] = &[
    TokenScope {
        value: "runs:read",
        field: "scope_runs_read",
        label: "Read runs and logs",
    },
    TokenScope {
        value: "runs:write",
        field: "scope_runs_write",
        label: "Start and cancel runs",
    },
    TokenScope {
        value: "workers:read",
        field: "scope_workers_read",
        label: "Read worker state",
    },
];

#[must_use]
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(home))
        .route("/login", get(login_page).post(login_start))
        .route("/signup", get(signup_page).post(signup_start))
        .route("/auth/callback", get(auth_callback))
        .route("/verify-email", get(verify_email))
        .route("/workspace", get(workspace))
        .route("/tokens", get(tokens_page).post(create_token))
        .route("/tokens/{token_id}/revoke", post(revoke_token))
        .route("/security", get(security_page))
        .route("/security/totp/enroll", post(totp_enroll))
        .route("/security/totp/confirm", post(totp_confirm))
        .route("/security/passkey/start", post(passkey_start))
        .merge(common::routes())
}

#[must_use]
pub fn router(state: AppState) -> Router {
    routes().with_state(state)
}

// ---- sign in / sign up -----------------------------------------------------

async fn home(Extension(context): Extension<RequestCtx>) -> Response {
    if context.actor.is_some() {
        return Redirect::to("/workspace").into_response();
    }
    Redirect::to("/login").into_response()
}

#[derive(Debug, Deserialize)]
pub struct NextQuery {
    #[serde(default)]
    pub next: Option<String>,
}

async fn login_page(Extension(context): Extension<RequestCtx>, Query(query): Query<NextQuery>) -> Response {
    if context.actor.is_some() {
        return Redirect::to("/workspace").into_response();
    }
    common::render(
        &context,
        &auth_meta("Personal account"),
        credential_form(&context, &query, false),
    )
}

async fn signup_page(Extension(context): Extension<RequestCtx>, Query(query): Query<NextQuery>) -> Response {
    if context.actor.is_some() {
        return Redirect::to("/workspace").into_response();
    }
    common::render(
        &context,
        &auth_meta("Create a personal account"),
        credential_form(&context, &query, true),
    )
}

fn auth_meta(title: &str) -> PageMeta {
    PageMeta::new(title, "Personal accounts for GHA Indie Worker").with_chat(ChatAudience::Visitor)
}

/// One form, two modes. Both hand off to shared-auth; neither collects a secret.
#[must_use]
pub fn credential_form(context: &RequestCtx, query: &NextQuery, signup: bool) -> Markup {
    let next = query
        .next
        .clone()
        .filter(|value| crate::session::is_safe_next(value))
        .unwrap_or_else(|| "/workspace".to_owned());
    let action = if signup { "/signup" } else { "/login" };
    let org_login = format!("{}/login", context.page().origin(Surface::Org));
    html! {
        section class="section" {
            p class="eyebrow" { "INDIVIDUALS" }
            h1 { @if signup { "Create a personal account" } @else { "Personal account" } }
            p class="lede" {
                "Your account is held by Shared Auth. This page never sees your password — it hands \
                 you over and takes you back once you are confirmed."
            }
            (forms::form(&forms::FormAction::post(action), &context.csrf_token, html! {
                input type="hidden" name="next" value=(next);
                (forms::text_field("account-email", "email", "Email", "email", "", "", true))
                (forms::actions(if signup { "Create account" } else { "Continue" }, "", ""))
            }))
            p {
                @if signup {
                    "Already have an account? " a class="text-link" href="/login" { "Sign in" } "."
                } @else {
                    "New here? " a class="text-link" href="/signup" { "Create an account" } "."
                }
            }
            p { "Signing in for a team? " a class="text-link" href=(org_login) { "Use your organization" } "." }
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CredentialForm {
    #[serde(default)]
    pub csrf_token: Option<String>,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub next: Option<String>,
}

async fn login_start(
    Extension(context): Extension<RequestCtx>,
    State(state): State<AppState>,
    Form(form): Form<CredentialForm>,
) -> Response {
    start_shared_auth(&context, &state, &form, "login").await
}

async fn signup_start(
    Extension(context): Extension<RequestCtx>,
    State(state): State<AppState>,
    Form(form): Form<CredentialForm>,
) -> Response {
    start_shared_auth(&context, &state, &form, "signup").await
}

async fn start_shared_auth(context: &RequestCtx, state: &AppState, form: &CredentialForm, intent: &str) -> Response {
    if let Err(error) = context.verify_form_csrf(&state.csrf, form.csrf_token.as_deref()) {
        return common::error_page(context, &error);
    }
    let Some(base) = state.config.shared_auth.base.as_deref() else {
        return common::error_page(context, &WebError::Unavailable);
    };
    let next = form
        .next
        .clone()
        .filter(|value| crate::session::is_safe_next(value))
        .unwrap_or_else(|| "/workspace".to_owned());
    let redirect_uri = format!("{}/auth/callback", context.page().origin(Surface::User));
    let target = format!(
        "{}/auth/authorize?surface=user&intent={intent}&redirect_uri={}&state={}&login_hint={}",
        base.trim_end_matches('/'),
        crate::session::percent_encode_path(&redirect_uri),
        crate::session::percent_encode_path(&next),
        crate::session::percent_encode_path(form.email.trim()),
    );
    Redirect::to(&target).into_response()
}

#[derive(Debug, Deserialize)]
pub struct CallbackQuery {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
}

async fn auth_callback(
    Extension(context): Extension<RequestCtx>,
    State(state): State<AppState>,
    Query(query): Query<CallbackQuery>,
) -> Response {
    crate::session::complete_exchange(&state, &context, query.code.as_deref(), query.state.as_deref()).await
}

/// Shown after sign-up, while the address is unconfirmed.
async fn verify_email(Extension(context): Extension<RequestCtx>) -> Response {
    let meta = PageMeta::new(
        "Confirm your email",
        "Confirm your address to finish setting up your account",
    )
    .with_chat(ChatAudience::Visitor);
    let body = html! {
        section class="section" {
            p class="eyebrow" { "ONE MORE STEP" }
            h1 { "Confirm your email" }
            (components::alert(
                components::Tone::Neutral,
                "Check your inbox",
                "We sent a confirmation link. Open it on this device and you will land back here signed in.",
            ))
            p { "Wrong address? " a class="text-link" href="/signup" { "Start again" } "." }
        }
    };
    common::render(&context, &meta, body)
}

// ---- workspace -------------------------------------------------------------

async fn workspace(Extension(context): Extension<RequestCtx>, State(state): State<AppState>) -> Response {
    let Some(actor) = context.actor.clone() else {
        return crate::session::login_redirect(context.surface, &context.base_domain, Some("/workspace"));
    };
    let runs = state.repo.runs(context.bearer(), None, 10).await.unwrap_or_default();
    let meta = PageMeta::new("Workspace", "Your personal runs and settings")
        .active("workspace")
        .with_chat(ChatAudience::Customer);
    let page = context.page();
    let body = html! {
        section class="section" {
            p class="eyebrow" { "PERSONAL" }
            h1 { "Workspace" }
            p class="lede" { (format!("Signed in as {}.", actor.display_name())) }
            div class="grid grid-3" {
                (components::stat("Recent runs", &runs.len().to_string(), "Yours only"))
                (components::stat("Step-up", if actor.stepped_up() { "Confirmed" } else { "Not confirmed" }, "Second factor for sensitive actions"))
                (components::stat("Organization", actor.org_id.as_deref().unwrap_or("None"), "Ask an owner for an invitation"))
            }
            div class="row" {
                a class="button" href=(format!("{}/runs", page.origin(Surface::App))) { "Open runs" }
                a class="button secondary" href="/tokens" { "API tokens" }
                a class="button secondary" href="/security" { "Security" }
            }
        }
        (components::section("01 / RUNS", "Recent runs", crate::hosts::app::run_table(&runs)))
    };
    common::render(&context, &meta, body)
}

// ---- API tokens ------------------------------------------------------------

async fn tokens_page(Extension(context): Extension<RequestCtx>, State(state): State<AppState>) -> Response {
    if context.actor.is_none() {
        return crate::session::login_redirect(context.surface, &context.base_domain, Some("/tokens"));
    }
    let meta = PageMeta::new("API tokens", "Personal tokens for the API and the CLI")
        .active("tokens")
        .with_chat(ChatAudience::Customer);
    let body = state
        .repo
        .tokens(context.bearer())
        .await
        .map(|tokens| token_page_body(&context, &tokens, None));
    common::render_result(&context, &meta, body)
}

#[derive(Debug, Default, Deserialize)]
pub struct CreateTokenForm {
    #[serde(default)]
    pub csrf_token: Option<String>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub scope_runs_read: Option<String>,
    #[serde(default)]
    pub scope_runs_write: Option<String>,
    #[serde(default)]
    pub scope_workers_read: Option<String>,
}

impl CreateTokenForm {
    /// The scopes that were actually ticked, in declaration order.
    #[must_use]
    pub fn selected_scopes(&self) -> Vec<String> {
        TOKEN_SCOPES
            .iter()
            .filter(|scope| self.field(scope.field).is_some())
            .map(|scope| scope.value.to_owned())
            .collect()
    }

    fn field(&self, name: &str) -> Option<&str> {
        match name {
            "scope_runs_read" => self.scope_runs_read.as_deref(),
            "scope_runs_write" => self.scope_runs_write.as_deref(),
            "scope_workers_read" => self.scope_workers_read.as_deref(),
            _ => None,
        }
    }
}

async fn create_token(
    Extension(context): Extension<RequestCtx>,
    State(state): State<AppState>,
    Form(form): Form<CreateTokenForm>,
) -> Response {
    if let Err(error) = context.verify_form_csrf(&state.csrf, form.csrf_token.as_deref()) {
        return common::error_page(&context, &error);
    }
    if context.actor.is_none() {
        return WebError::Unauthenticated.into_response();
    }
    let name = form.name.trim();
    if name.is_empty() {
        return common::error_page(
            &context,
            &WebError::BadRequest("Give the token a name you will recognise."),
        );
    }
    let scopes = form.selected_scopes();
    match state.repo.api().create_token(context.bearer(), name, &scopes).await {
        Ok(value) => {
            // The secret is returned exactly once, by the api-server. It is
            // rendered here and never stored, logged or put in a URL.
            let secret = value
                .get("token")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
            let tokens = state.repo.tokens(context.bearer()).await.unwrap_or_default();
            let meta = PageMeta::new("API tokens", "Personal tokens").active("tokens");
            common::render(&context, &meta, token_page_body(&context, &tokens, secret.as_deref()))
        }
        Err(error) => common::error_page(&context, &error),
    }
}

async fn revoke_token(
    Extension(context): Extension<RequestCtx>,
    State(state): State<AppState>,
    Path(token_id): Path<String>,
    Form(form): Form<common::CsrfForm>,
) -> Response {
    if let Err(error) = context.verify_form_csrf(&state.csrf, form.csrf_token.as_deref()) {
        return error.into_response();
    }
    match state.repo.api().revoke_token(context.bearer(), &token_id).await {
        Ok(_) => Redirect::to("/tokens").into_response(),
        Err(error) => error.into_response(),
    }
}

#[must_use]
pub fn token_page_body(context: &RequestCtx, tokens: &[TokenSummary], new_secret: Option<&str>) -> Markup {
    html! {
        section class="section" {
            p class="eyebrow" { "PERSONAL" }
            h1 { "API tokens" }
            @if let Some(secret) = new_secret {
                (components::alert(
                    components::Tone::Ok,
                    "Copy this token now",
                    "This is the only time it is shown. Store it in your secret manager, not in a repository.",
                ))
                pre class="logs" { (secret) }
            }
            (forms::form(&forms::FormAction::post("/tokens"), &context.csrf_token, html! {
                (forms::text_field("token-name", "name", "Token name", "text", "", "For example: laptop CLI", true))
                (forms::fieldset("Scopes", html! {
                    @for scope in TOKEN_SCOPES {
                        div class="field" {
                            label for=(scope.field) {
                                input id=(scope.field) type="checkbox" name=(scope.field) value=(scope.value);
                                " " (scope.label)
                            }
                        }
                    }
                }))
                (forms::actions("Create token", "", ""))
            }))
        }
        (components::section("01 / TOKENS", "Your tokens", token_table(context, tokens)))
    }
}

fn token_table(context: &RequestCtx, tokens: &[TokenSummary]) -> Markup {
    let rows = tokens
        .iter()
        .map(|token| {
            let action = format!("/tokens/{}/revoke", token.id);
            tables::row(vec![
                tables::cell(&token.name),
                tables::cell(&token.prefix),
                tables::cell(&token.scopes.join(", ")),
                tables::cell(token.last_used_at.as_deref().unwrap_or("never")),
                tables::markup_cell(forms::form(
                    &forms::FormAction::post(action.as_str()),
                    &context.csrf_token,
                    html! { button type="submit" class="button danger" { "Revoke" } },
                )),
            ])
        })
        .collect();
    tables::table(
        "Tokens",
        &[
            tables::Column::text("Name"),
            tables::Column::text("Prefix"),
            tables::Column::text("Scopes"),
            tables::Column::text("Last used"),
            tables::Column::text(""),
        ],
        rows,
    )
}

// ---- step-up ---------------------------------------------------------------

/// What the security page knows. Built by the handler from shared-auth.
#[derive(Clone, Debug, Default)]
pub struct SecurityView {
    pub factors: Vec<Factor>,
    pub enrollment: Option<TotpEnrollment>,
    pub ceremony: Option<CeremonyStart>,
    pub notice: Option<String>,
    pub stepped_up: bool,
}

async fn security_page(Extension(context): Extension<RequestCtx>, State(state): State<AppState>) -> Response {
    let Some(actor) = context.actor.clone() else {
        return crate::session::login_redirect(context.surface, &context.base_domain, Some("/security"));
    };
    let factors = match state.auth.shared_auth() {
        Some(client) => client.factors(&actor.access_token).await.unwrap_or_default(),
        None => Vec::new(),
    };
    let view = SecurityView {
        factors,
        stepped_up: actor.stepped_up(),
        ..SecurityView::default()
    };
    render_security(&context, &view)
}

async fn totp_enroll(
    Extension(context): Extension<RequestCtx>,
    State(state): State<AppState>,
    Form(form): Form<common::CsrfForm>,
) -> Response {
    if let Err(error) = context.verify_form_csrf(&state.csrf, form.csrf_token.as_deref()) {
        return common::error_page(&context, &error);
    }
    let Some(actor) = context.actor.clone() else {
        return WebError::Unauthenticated.into_response();
    };
    let Some(client) = state.auth.shared_auth() else {
        return common::error_page(&context, &WebError::Unavailable);
    };
    let mut view = SecurityView {
        stepped_up: actor.stepped_up(),
        ..SecurityView::default()
    };
    match client.enroll_totp(&actor.access_token, Some("indiebuild.dev")).await {
        Ok(enrollment) => {
            view.enrollment = Some(enrollment);
            view.notice = Some("Scan the secret in your authenticator, then enter the six-digit code.".to_owned());
        }
        Err(_) => view.notice = Some("Shared Auth could not start enrolment. Nothing changed.".to_owned()),
    }
    view.factors = client.factors(&actor.access_token).await.unwrap_or_default();
    render_security(&context, &view)
}

#[derive(Debug, Deserialize)]
pub struct TotpConfirmForm {
    #[serde(default)]
    pub csrf_token: Option<String>,
    #[serde(default)]
    pub factor_id: String,
    #[serde(default)]
    pub code: String,
}

/// A confirmed factor raises the session's assurance, so the new `acr` and
/// access token are written back into the cookie immediately.
async fn totp_confirm(
    Extension(context): Extension<RequestCtx>,
    State(state): State<AppState>,
    Form(form): Form<TotpConfirmForm>,
) -> Response {
    if let Err(error) = context.verify_form_csrf(&state.csrf, form.csrf_token.as_deref()) {
        return common::error_page(&context, &error);
    }
    let Some(actor) = context.actor.clone() else {
        return WebError::Unauthenticated.into_response();
    };
    let Some(client) = state.auth.shared_auth() else {
        return common::error_page(&context, &WebError::Unavailable);
    };
    match client
        .confirm_totp(&actor.access_token, form.factor_id.trim(), form.code.trim())
        .await
    {
        Ok(step_up) => {
            let session = crate::session::SessionData {
                sub: actor.sub.clone(),
                access_token: step_up.access_token,
                expires_at: i64::try_from(step_up.expires_at).unwrap_or(i64::MAX),
                email: actor.email.clone(),
                org_id: actor.org_id.clone(),
                roles: actor.roles.clone(),
                scopes: actor.scopes.clone(),
                acr: step_up.acr,
                surface: context.surface.label().to_owned(),
            };
            match state.sessions.encode(&session) {
                Some(encoded) => crate::session::with_cookie(
                    Redirect::to("/security").into_response(),
                    state.sessions.set_cookie(&encoded),
                ),
                None => common::error_page(&context, &WebError::Internal),
            }
        }
        Err(_) => {
            let view = SecurityView {
                factors: client.factors(&actor.access_token).await.unwrap_or_default(),
                notice: Some("That code was not accepted. Try the next one your authenticator shows.".to_owned()),
                stepped_up: actor.stepped_up(),
                ..SecurityView::default()
            };
            render_security(&context, &view)
        }
    }
}

/// Starts a WebAuthn registration ceremony. The ceremony itself is finished by
/// the island bundle, which posts the credential to the api-server.
async fn passkey_start(
    Extension(context): Extension<RequestCtx>,
    State(state): State<AppState>,
    Form(form): Form<common::CsrfForm>,
) -> Response {
    if let Err(error) = context.verify_form_csrf(&state.csrf, form.csrf_token.as_deref()) {
        return common::error_page(&context, &error);
    }
    let Some(actor) = context.actor.clone() else {
        return WebError::Unauthenticated.into_response();
    };
    let Some(client) = state.auth.shared_auth() else {
        return common::error_page(&context, &WebError::Unavailable);
    };
    let mut view = SecurityView {
        stepped_up: actor.stepped_up(),
        ..SecurityView::default()
    };
    match client
        .start_passkey_registration(&actor.access_token, Some("indiebuild.dev"))
        .await
    {
        Ok(ceremony) => view.ceremony = Some(ceremony),
        Err(_) => view.notice = Some("Shared Auth could not start a passkey ceremony. Nothing changed.".to_owned()),
    }
    view.factors = client.factors(&actor.access_token).await.unwrap_or_default();
    render_security(&context, &view)
}

fn render_security(context: &RequestCtx, view: &SecurityView) -> Response {
    let meta = PageMeta::new("Security", "Two-factor methods and passkeys")
        .active("security")
        .with_chat(ChatAudience::Customer);
    common::render(context, &meta, security_body(context, view))
}

/// Pure, so the step-up markup is snapshot-testable without shared-auth.
#[must_use]
pub fn security_body(context: &RequestCtx, view: &SecurityView) -> Markup {
    let page = context.page();
    html! {
        section class="section" {
            p class="eyebrow" { "SECURITY" }
            h1 { "Security" }
            @if view.stepped_up {
                (components::alert(components::Tone::Ok, "This session is stepped up", "Sensitive actions are unlocked for this session."))
            }
            @if let Some(notice) = &view.notice {
                (components::alert(components::Tone::Warn, "Heads up", notice))
            }
            (factor_table(&view.factors))
        }

        section class="section" {
            p class="section-number" { "01 / AUTHENTICATOR" }
            h2 { "Time-based one-time codes" }
            @match &view.enrollment {
                Some(enrollment) => {
                    p { "Add this secret to your authenticator, then confirm with the code it shows." }
                    (components::card("SECRET", html! {
                        pre class="logs" { (enrollment.secret_base32) }
                        p class="hint" { (enrollment.otpauth_uri) }
                    }))
                    (forms::form(&forms::FormAction::post("/security/totp/confirm"), &context.csrf_token, html! {
                        input type="hidden" name="factor_id" value=(enrollment.factor_id);
                        (forms::text_field("totp-code", "code", "Six-digit code", "text", "", "From your authenticator app.", true))
                        (forms::actions("Confirm", "", ""))
                    }))
                }
                None => {
                    p { "Add a second factor so a stolen password is not enough." }
                    (forms::form(&forms::FormAction::post("/security/totp/enroll"), &context.csrf_token, html! {
                        (forms::actions("Set up an authenticator", "", ""))
                    }))
                }
            }
        }

        section class="section" {
            p class="section-number" { "02 / PASSKEYS" }
            h2 { "Passkeys" }
            @if loader::islands_enabled() {
                (forms::form(&forms::FormAction::post("/security/passkey/start"), &context.csrf_token, html! {
                    (forms::actions("Add a passkey", "", ""))
                }))
                @if let Some(ceremony) = &view.ceremony {
                    (loader::island_mount(&page, "passkey-registration", html! {
                        p { "Your browser will ask you to confirm. If nothing happens, your device may not support passkeys." }
                    }))
                    // The ceremony options travel as an escaped attribute, never as
                    // an inline script body, so nothing in them can close a tag.
                    div id="passkey-ceremony" hidden="hidden"
                        data-ceremony=(serde_json::to_string(&ceremony.options).unwrap_or_else(|_| "{}".to_owned())) {}
                }
            } @else {
                (components::alert(
                    components::Tone::Neutral,
                    "Passkeys need the app bundle",
                    "This deployment was built without the client island that runs the WebAuthn ceremony. Use an authenticator app for now.",
                ))
            }
        }
    }
}

fn factor_table(factors: &[Factor]) -> Markup {
    let rows = factors
        .iter()
        .map(|factor| {
            tables::row(vec![
                tables::cell(factor.label.as_deref().unwrap_or(&factor.kind)),
                tables::cell(&factor.kind),
                tables::markup_cell(if factor.enabled {
                    components::badge("enabled", components::Tone::Ok)
                } else {
                    components::badge("disabled", components::Tone::Warn)
                }),
                tables::cell(factor.confirmed_at.as_deref().unwrap_or("not confirmed")),
                tables::cell(factor.last_used_at.as_deref().unwrap_or("never")),
            ])
        })
        .collect();
    tables::table(
        "Factors",
        &[
            tables::Column::text("Label"),
            tables::Column::text("Kind"),
            tables::Column::text("State"),
            tables::Column::text("Confirmed"),
            tables::Column::text("Last used"),
        ],
        rows,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> RequestCtx {
        RequestCtx {
            surface: Surface::User,
            host: "user.indiebuild.dev".into(),
            base_domain: "indiebuild.dev".into(),
            path: "/login".into(),
            nonce: "n0nce".into(),
            csrf_token: "tok3n".into(),
            csrf: crate::csrf::CsrfCheck::NotRequired,
            actor: None,
            is_htmx: false,
            subject: "anonymous".into(),
            release_manifest_url: None,
            chat_enabled: true,
        }
    }

    #[test]
    fn login_and_signup_share_a_form_but_not_a_call_to_action() {
        let query = NextQuery { next: None };
        let login = credential_form(&context(), &query, false).into_string();
        assert!(login.contains(r#"hx-post="/login""#));
        assert!(login.contains("Continue"));
        assert!(login.contains("Create an account"));

        let signup = credential_form(&context(), &query, true).into_string();
        assert!(signup.contains(r#"hx-post="/signup""#));
        assert!(signup.contains("Create account"));
    }

    #[test]
    fn the_personal_form_points_teams_at_the_organization_host() {
        let html = credential_form(&context(), &NextQuery { next: None }, false).into_string();
        assert!(html.contains("https://org.indiebuild.dev/login"));
    }

    #[test]
    fn an_unsafe_next_value_is_dropped_rather_than_echoed() {
        let query = NextQuery {
            next: Some("https://evil.example/steal".into()),
        };
        let html = credential_form(&context(), &query, false).into_string();
        assert!(!html.contains("evil.example"));
        assert!(html.contains(r#"name="next" value="/workspace""#));
    }

    #[test]
    fn the_token_page_shows_a_new_secret_exactly_once_and_never_stores_it() {
        let html = token_page_body(&context(), &[], Some("giw_live_secret")).into_string();
        assert!(html.contains("giw_live_secret"));
        assert!(html.contains("only time it is shown"));

        let without = token_page_body(&context(), &[], None).into_string();
        assert!(!without.contains("giw_live_secret"));
    }

    #[test]
    fn every_token_control_carries_the_csrf_token() {
        let tokens = vec![TokenSummary {
            id: "tok_1".into(),
            name: "laptop".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            last_used_at: None,
            scopes: vec!["runs:read".into()],
            prefix: "giw_live_9f3a".into(),
        }];
        let html = token_page_body(&context(), &tokens, None).into_string();
        assert!(html.contains(r#"hx-post="/tokens/tok_1/revoke""#));
        assert_eq!(html.matches(r#"name="csrf_token" value="tok3n""#).count(), 2);
    }

    #[test]
    fn only_declared_scopes_are_offered() {
        let html = token_page_body(&context(), &[], None).into_string();
        for scope in TOKEN_SCOPES {
            assert!(html.contains(scope.value), "missing scope {}", scope.value);
            assert!(html.contains(scope.label));
        }
        assert!(!html.contains("admin"));
    }

    #[test]
    fn the_security_page_offers_enrolment_before_a_factor_exists() {
        let html = security_body(&context(), &SecurityView::default()).into_string();
        assert!(html.contains(r#"hx-post="/security/totp/enroll""#));
        assert!(!html.contains(r#"hx-post="/security/totp/confirm""#));
    }

    #[test]
    fn an_enrolment_shows_the_secret_and_asks_for_a_code() {
        let view = SecurityView {
            enrollment: Some(TotpEnrollment {
                factor_id: "f1".into(),
                secret_base32: "JBSWY3DPEHPK3PXP".into(),
                otpauth_uri: "otpauth://totp/indiebuild.dev".into(),
                threefa_import_uri: "threefa://import".into(),
            }),
            ..SecurityView::default()
        };
        let html = security_body(&context(), &view).into_string();
        assert!(html.contains("JBSWY3DPEHPK3PXP"));
        assert!(html.contains(r#"hx-post="/security/totp/confirm""#));
        assert!(html.contains(r#"name="factor_id" value="f1""#));
    }

    #[test]
    fn a_stepped_up_session_says_so() {
        let view = SecurityView {
            stepped_up: true,
            ..SecurityView::default()
        };
        assert!(security_body(&context(), &view)
            .into_string()
            .contains("This session is stepped up"));
    }

    #[test]
    fn passkeys_are_honest_about_needing_the_island_bundle() {
        let html = security_body(&context(), &SecurityView::default()).into_string();
        if loader::islands_enabled() {
            assert!(html.contains(r#"hx-post="/security/passkey/start""#));
        } else {
            assert!(html.contains("Passkeys need the app bundle"));
            assert!(!html.contains("/security/passkey/start"));
        }
    }

    #[test]
    fn factors_render_without_leaking_an_identifier() {
        let factors = vec![Factor {
            factor_id: "f_secret_id".into(),
            kind: "totp".into(),
            label: Some("Phone".into()),
            enabled: true,
            confirmed_at: Some("2026-01-01T00:00:00Z".into()),
            last_used_at: None,
            created_at: "2026-01-01T00:00:00Z".into(),
        }];
        let html = factor_table(&factors).into_string();
        assert!(html.contains("Phone"));
        assert!(html.contains("totp"));
        assert!(!html.contains("f_secret_id"));
    }
}
