#![forbid(unsafe_code)]
//! The router, and the one place a response is assembled.
//!
//! Reading order:
//!
//! * [`router`] — every route this binary answers. There is exactly one `GET` handler ([`show`]);
//!   which page it renders is decided by [`crate::present::page_for`], a pure function, so the
//!   routing table and the page table cannot drift apart.
//! * [`respond`] — the only function that turns markup into an HTTP response, and therefore the
//!   only place security headers and cookies are attached. A handler cannot forget them because a
//!   handler does not have the option.
//! * [`accept_post`] — the shared preamble for every state-changing request: resolve the surface,
//!   check the `Origin`, check CSRF, parse the form. A `POST` handler starts *after* all four have
//!   passed.

pub mod app;
pub mod forms;
pub mod health;
pub mod home;
pub mod layout;
pub mod org;
pub mod user;

use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use axum::Router;
use gha_indie_worker_lib_core::runtime::surface::{self, Disposition};
use maud::Markup;

use crate::csrf::{self, CookieLife};
use crate::error::WebError;
use crate::policy::{self, PolicyKind};
use crate::present::{Face, Page};
use crate::state::{AppState, Ctx};
use crate::{assets, auth};

/// Every route. One `GET`, a handful of `POST`s, the socket, the assets, and the probes.
#[must_use]
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(health::healthz))
        .route("/readyz", get(health::readyz))
        .route("/assets/{file}", get(asset))
        .route("/ws", any(crate::ws::upgrade))
        // Sign-in is the only GET that changes anything, because a magic link arrives as a link.
        // It is single use and short lived; see `auth::MagicLinkStore`.
        .route("/auth/callback", get(user::callback))
        .route("/auth/magic-link", post(user::request_magic_link))
        .route("/auth/signout", post(user::sign_out))
        .route("/org/new", post(org::create))
        .route("/org/invitations", post(org::invite))
        .route("/org/invitations/revoke", post(org::revoke))
        .route("/org/invitations/resend", post(org::resend))
        .route("/org/members/role", post(org::change_role))
        .route("/org/members/remove", post(org::remove_member))
        .route("/org/domains", post(org::claim_domain))
        .route("/org/domains/verify", post(org::verify_domain))
        .route("/org/domains/join", post(org::set_domain_join))
        .fallback(show)
        .with_state(state)
}

/// The single `GET` handler.
///
/// # Errors
/// [`WebError::NotFound`] for a host that is not ours or a surface this binary refuses.
pub async fn show(
    State(state): State<AppState>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, WebError> {
    let ctx = state.context(&headers).ok_or(WebError::NotFound)?;

    // `surface::dispose` is the authority on what happens to a request for a given host, and it is
    // consulted here rather than re-derived: it is what makes `admin.` a 404 from this binary.
    match surface::dispose(&ctx.host, &state.config.apex, ctx.authenticated()) {
        Disposition::NotFound => return Err(WebError::NotFound),
        Disposition::RedirectToLogin(login) => {
            let target = layout::surface_url(
                &state.config.apex,
                state.config.cookies_are_secure(),
                login.label(),
                "/",
            );
            return Ok(redirect(&state, &ctx, &target));
        }
        Disposition::Serve(_) => {}
    }

    let page = crate::present::page_for(ctx.face, uri.path(), ctx.authenticated());
    let nonce = auth::random_nonce();
    let query = csrf::parse_form(uri.query().unwrap_or_default());

    // Dispatch on the *page*, not on the surface. `m.` is a login door when anonymous and the
    // application shell when not, so a surface-keyed match would hand an anonymous mobile visitor
    // to the application renderer and get a 404 for a page that exists. Matching on `Page` is also
    // total: a page added to `present` fails to compile until something renders it.
    let body = match &page {
        Page::MarketingHome | Page::MarketingPricing => home::render(&state, &ctx, &page),
        Page::UserSignIn
        | Page::UserSignUp
        | Page::UserCheckEmail
        | Page::UserCallback
        | Page::UserSettings
        | Page::UserSwitchToOrg => user::render(&state, &ctx, &page, &query),
        Page::OrgSignIn
        | Page::OrgCreate
        | Page::OrgMembers
        | Page::OrgInvitations
        | Page::OrgDomains
        | Page::OrgSso
        | Page::OrgSettings => org::render(&state, &ctx, &page),
        Page::AppDashboard
        | Page::AppRuns
        | Page::AppRunDetail(_)
        | Page::AppRunners
        | Page::AppCaches
        | Page::AppSettings => app::render(&state, &ctx, &page),
        Page::NotFound => not_found(&ctx),
    };

    let status = if matches!(page, Page::NotFound) {
        StatusCode::NOT_FOUND
    } else {
        StatusCode::OK
    };
    Ok(respond(&state, &ctx, &page, &nonce, status, body))
}

/// Serve one embedded asset.
///
/// # Errors
/// [`WebError::NotFound`] for anything not in the registry. There is no filesystem here to walk,
/// so a traversal attempt cannot reach one.
pub async fn asset(Path(file): Path<String>) -> Result<Response, WebError> {
    let path = format!("/assets/{file}");
    let (asset, integrity) = assets::assets().get(&path).ok_or(WebError::NotFound)?;
    let mut response = (StatusCode::OK, asset.bytes).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(asset.content_type),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(policy::ASSET_CACHE_CONTROL),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    // The digest the page's `integrity` attribute will be checked against, as an `ETag`: the same
    // bytes always produce the same tag, and different bytes never share one.
    if let Ok(tag) = HeaderValue::from_str(&format!("\"{integrity}\"")) {
        headers.insert(header::ETAG, tag);
    }
    Ok(response)
}

/// Assemble a full-page response: markup, security headers, cookies.
#[must_use]
pub fn respond(
    state: &AppState,
    ctx: &Ctx,
    page: &Page,
    nonce: &str,
    status: StatusCode,
    body: Markup,
) -> Response {
    let chrome = layout::Chrome {
        ctx,
        page,
        nonce,
        apex: &state.config.apex,
        secure: state.config.cookies_are_secure(),
        inline_script: app::inline_script(ctx, page, state),
    };
    let markup = layout::shell(&chrome, body);
    let mut response = (status, markup).into_response();
    harden(state, ctx, nonce, &mut response);
    response
}

/// Assemble a fragment response — an htmx swap. Same headers, no shell.
#[must_use]
pub fn fragment(state: &AppState, ctx: &Ctx, status: StatusCode, body: Markup) -> Response {
    let mut response = (status, body).into_response();
    // A fragment carries no script, so its policy names none: it is inserted into a document whose
    // own policy already applies, and this one is belt and braces for a fragment fetched directly.
    harden(state, ctx, "", &mut response);
    response
}

/// A redirect that still carries the hardening headers and any cookie that was due.
#[must_use]
pub fn redirect(state: &AppState, ctx: &Ctx, target: &str) -> Response {
    let mut response = StatusCode::SEE_OTHER.into_response();
    if let Ok(value) = HeaderValue::from_str(target) {
        response.headers_mut().insert(header::LOCATION, value);
    }
    harden(state, ctx, "", &mut response);
    response
}

/// htmx honours `HX-Redirect` on a fragment response; a plain browser needs a 303. Sending both
/// means one handler serves both clients without branching on a header it does not control.
#[must_use]
pub fn redirect_fragment(state: &AppState, ctx: &Ctx, target: &str) -> Response {
    let mut response = redirect(state, ctx, target);
    if let Ok(value) = HeaderValue::from_str(target) {
        response
            .headers_mut()
            .insert(HeaderName::from_static("hx-redirect"), value);
    }
    response
}

fn policy_kind(face: Face) -> PolicyKind {
    match face {
        Face::Marketing => PolicyKind::Marketing,
        Face::User | Face::Org => PolicyKind::Interactive,
        Face::App | Face::Mobile => PolicyKind::Application,
    }
}

/// Attach the security headers and any cookie the request is owed.
fn harden(state: &AppState, ctx: &Ctx, nonce: &str, response: &mut Response) {
    let secure = state.config.cookies_are_secure();
    let headers = response.headers_mut();
    for (name, value) in policy::security_headers(policy_kind(ctx.face), nonce, &ctx.host, secure) {
        if let Ok(value) = HeaderValue::from_str(&value) {
            headers.insert(HeaderName::from_static(name), value);
        }
    }
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(policy::PRIVATE_CACHE_CONTROL),
    );
    headers.insert(header::VARY, HeaderValue::from_static("Cookie, HX-Request"));
    if ctx.refresh_csrf_cookie {
        let cookie = csrf::set_cookie(
            csrf::COOKIE_NAME,
            &ctx.csrf_token,
            state.config.cookie_domain(),
            CookieLife::Session,
            secure,
            true,
        );
        if let Ok(value) = HeaderValue::from_str(&cookie) {
            headers.append(header::SET_COOKIE, value);
        }
    }
}

/// Add a `Set-Cookie` to a response that has already been hardened.
pub fn attach_cookie(response: &mut Response, cookie: &str) {
    if let Ok(value) = HeaderValue::from_str(cookie) {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
}

/// What a state-changing request looks like once it has been accepted: the resolved request
/// context, and the decoded form body.
pub type Submission = (Ctx, Vec<(String, String)>);

/// The body of a 404, rendered inside the shell of whatever surface the visitor reached.
///
/// It names the surface, because "there is nothing at this address on `org.`" is a more useful
/// thing to read than "not found" when the same path exists on `app.`
#[must_use]
pub fn not_found(ctx: &Ctx) -> Markup {
    maud::html! {
        div class="wrap-narrow stack" {
            p class="eyebrow" { "404" }
            h1 { "There is nothing at this address." }
            p class="lede" {
                "You are on the " code class="mono" { (ctx.face.label()) } " surface. The page you \
                 wanted may live on another one."
            }
            p { a class="button button-primary" href="/" { "Back to the start" } }
        }
    }
}

/// The shared preamble for a state-changing request.
///
/// Order matters and is deliberate: the surface is resolved first (so an admin host never reaches
/// a mutation), then the `Origin` (cheap, and refuses a cross-site post before any token is
/// compared), then CSRF, then the body is parsed. Nothing that costs work happens before something
/// that can refuse.
///
/// # Errors
/// [`WebError::NotFound`], [`WebError::Forbidden`], [`WebError::Csrf`].
pub fn accept_post(
    state: &AppState,
    headers: &HeaderMap,
    body: &str,
) -> Result<Submission, WebError> {
    let ctx = state.context(headers).ok_or(WebError::NotFound)?;
    if matches!(
        surface::dispose(&ctx.host, &state.config.apex, ctx.authenticated()),
        Disposition::NotFound
    ) {
        return Err(WebError::NotFound);
    }
    let secure = state.config.cookies_are_secure();
    if !csrf::origin_is_ours(ctx.origin.as_deref(), &ctx.host, secure) {
        tracing::warn!(host = %ctx.host, origin = ?ctx.origin, "cross-origin form post refused");
        return Err(WebError::Forbidden);
    }

    let form = csrf::parse_form(body);
    let cookie_header = headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok());
    let submitted = csrf::form_value(&form, csrf::FIELD_NAME).or_else(|| {
        headers
            .get(csrf::HEADER_NAME)
            .and_then(|value| value.to_str().ok())
    });
    csrf::verify(
        csrf::cookie_from(cookie_header, csrf::COOKIE_NAME),
        submitted,
        ctx.bound_token(),
    )?;
    Ok((ctx, form))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> AppState {
        let mut config = crate::config::WebConfig::from_env();
        config.apex = "indiebuild.dev".to_owned();
        config.public_url = "https://org.indiebuild.dev".to_owned();
        AppState::new(config)
    }

    fn post_headers(host: &str, origin: Option<&str>, cookie: Option<&str>) -> HeaderMap {
        let mut map = HeaderMap::new();
        map.insert(header::HOST, host.parse().unwrap());
        if let Some(origin) = origin {
            map.insert(header::ORIGIN, origin.parse().unwrap());
        }
        if let Some(cookie) = cookie {
            map.insert(header::COOKIE, cookie.parse().unwrap());
        }
        map
    }

    #[test]
    fn a_post_needs_the_right_host_origin_and_token_and_refuses_in_that_order() {
        let state = state();
        let token = auth::random_token();
        let cookie = format!("giw_csrf={token}");
        let body = format!("csrf_token={token}&email=jo%40acme.test");

        // Everything correct.
        let (ctx, form) = accept_post(
            &state,
            &post_headers(
                "org.indiebuild.dev",
                Some("https://org.indiebuild.dev"),
                Some(&cookie),
            ),
            &body,
        )
        .expect("a well-formed post");
        assert_eq!(ctx.face, Face::Org);
        assert_eq!(csrf::form_value(&form, "email"), Some("jo@acme.test"));

        // An admin host never reaches a mutation, whatever else it carries.
        assert!(matches!(
            accept_post(
                &state,
                &post_headers(
                    "admin.indiebuild.dev",
                    Some("https://admin.indiebuild.dev"),
                    Some(&cookie)
                ),
                &body,
            ),
            Err(WebError::NotFound)
        ));

        // A cross-site origin is refused before the token is even looked at.
        assert!(matches!(
            accept_post(
                &state,
                &post_headers(
                    "org.indiebuild.dev",
                    Some("https://evil.test"),
                    Some(&cookie)
                ),
                &body,
            ),
            Err(WebError::Forbidden)
        ));

        // A missing Origin is refused, not tolerated.
        assert!(matches!(
            accept_post(
                &state,
                &post_headers("org.indiebuild.dev", None, Some(&cookie)),
                &body
            ),
            Err(WebError::Forbidden)
        ));

        // The cookie and the field must agree.
        let other = auth::random_token();
        assert!(matches!(
            accept_post(
                &state,
                &post_headers(
                    "org.indiebuild.dev",
                    Some("https://org.indiebuild.dev"),
                    Some(&format!("giw_csrf={other}"))
                ),
                &body,
            ),
            Err(WebError::Csrf(_))
        ));

        // And the field must be there at all.
        assert!(matches!(
            accept_post(
                &state,
                &post_headers(
                    "org.indiebuild.dev",
                    Some("https://org.indiebuild.dev"),
                    Some(&cookie)
                ),
                "email=jo%40acme.test",
            ),
            Err(WebError::Csrf(_))
        ));
    }

    #[test]
    fn the_header_form_of_the_token_is_accepted_for_htmx_requests_with_no_body() {
        let state = state();
        let token = auth::random_token();
        let mut headers = post_headers(
            "org.indiebuild.dev",
            Some("https://org.indiebuild.dev"),
            Some(&format!("giw_csrf={token}")),
        );
        headers.insert(csrf::HEADER_NAME, token.parse().unwrap());
        assert!(accept_post(&state, &headers, "").is_ok());
    }

    #[test]
    fn each_face_gets_the_policy_written_for_it() {
        assert_eq!(policy_kind(Face::Marketing), PolicyKind::Marketing);
        assert_eq!(policy_kind(Face::User), PolicyKind::Interactive);
        assert_eq!(policy_kind(Face::Org), PolicyKind::Interactive);
        assert_eq!(policy_kind(Face::App), PolicyKind::Application);
        assert_eq!(policy_kind(Face::Mobile), PolicyKind::Application);
    }
}
