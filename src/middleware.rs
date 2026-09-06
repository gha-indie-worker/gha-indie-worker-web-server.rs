#![forbid(unsafe_code)]

//! Request context and the ores-middleware installation.
//!
//! Two layers, in this order:
//!
//! 1. **ores-middleware** (outermost) — the fleet's fixed pipeline: proxy trust →
//!    TLS policy → ids → context → ip policy → auth → rate limit → idempotency →
//!    handler → security headers / compression / metrics / sync-observe. Its auth
//!    and rate-limit hooks are filled by [`crate::auth::WebAuthVerifier`] and
//!    [`crate::rate_limit::OpaqueRateLimiter`], so this server shares the fleet's
//!    posture instead of inventing one.
//! 2. **[`request_context`]** — the web-specific part ores-middleware cannot
//!    know about: which [`Surface`] the `Host` resolved to, the per-request CSP
//!    nonce, the double-submit CSRF token, and the resolved [`Actor`].
//!
//! CSRF is checked here, before dispatch, because a rejected request must never
//! reach a handler that might act on its body.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::extract::{ConnectInfo, Request, State};
use axum::http::header::{HeaderName, HeaderValue, CACHE_CONTROL, SET_COOKIE};
use axum::http::HeaderMap;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Router;
use next_loggers::Logger;
use ores_middleware::BootstrapError;

use crate::auth::{bearer_token, WebAuthVerifier};
use crate::csrf::{self, CsrfCheck, ANONYMOUS_SUBJECT};
use crate::error::WebError;
use crate::hosts::Surface;
use crate::rate_limit::OpaqueRateLimiter;
use crate::session::{cookie_value, now_unix, Actor};
use crate::state::AppState;
use crate::ui::layout::PageContext;

/// Everything resolved once per request and shared by every handler.
#[derive(Clone, Debug)]
pub struct RequestCtx {
    pub surface: Surface,
    /// The effective host, after trusted-proxy forwarding.
    pub host: String,
    pub base_domain: String,
    /// Path only — never the query string, which may carry a `next` value.
    pub path: String,
    /// Per-request CSP nonce.
    pub nonce: String,
    /// Double-submit CSRF token for [`Self::subject`].
    pub csrf_token: String,
    /// Whether the CSRF token still has to be checked against a form field.
    pub csrf: CsrfCheck,
    pub actor: Option<Actor>,
    /// The request came from htmx, so a fragment is the right answer.
    pub is_htmx: bool,
    /// Subject the CSRF token is bound to: the user id, or `anonymous`.
    pub subject: String,
    pub release_manifest_url: Option<String>,
    pub chat_enabled: bool,
}

impl RequestCtx {
    /// The context the maud layer sees.
    #[must_use]
    pub fn page(&self) -> PageContext {
        PageContext {
            surface: self.surface,
            base_domain: self.base_domain.clone(),
            nonce: self.nonce.clone(),
            csrf_token: self.csrf_token.clone(),
            actor: self.actor.clone(),
            path: self.path.clone(),
            release_manifest_url: self.release_manifest_url.clone(),
            chat_enabled: self.chat_enabled,
        }
    }

    #[must_use]
    pub fn bearer(&self) -> Option<&str> {
        self.actor.as_ref().map(|actor| actor.access_token.as_str())
    }

    #[must_use]
    pub fn org_id(&self) -> Option<&str> {
        self.actor.as_ref().and_then(|actor| actor.org_id.as_deref())
    }

    /// Verifies a `csrf_token` form field for a request whose header was absent.
    /// Safe methods and header-verified requests short-circuit to `Ok`.
    pub fn verify_form_csrf(&self, key: &csrf::CsrfKey, field: Option<&str>) -> Result<(), WebError> {
        match self.csrf {
            CsrfCheck::NotRequired | CsrfCheck::HeaderAccepted => Ok(()),
            CsrfCheck::Rejected => Err(WebError::CsrfRejected),
            CsrfCheck::DeferredToForm => match field {
                Some(token) if key.verify(token, &self.subject) => Ok(()),
                _ => Err(WebError::CsrfRejected),
            },
        }
    }
}

/// Resolves the context, rejects a bad CSRF token, then stamps security headers
/// onto the response.
pub async fn request_context(State(state): State<AppState>, mut request: Request, next: Next) -> Response {
    let peer = peer_ip(&request);
    let headers = request.headers().clone();
    let method = request.method().clone();
    let path = request.uri().path().to_owned();

    let surface = state.hosts.resolve(&headers, peer);
    let host = state.hosts.effective_host(&headers, peer).unwrap_or_default();
    let actor = resolve_actor(&state, &headers, surface).await;
    let subject = actor
        .as_ref()
        .map_or_else(|| ANONYMOUS_SUBJECT.to_owned(), |actor| actor.sub.clone());

    // Reuse the visitor's token when it is still valid for this subject, so a
    // sign-in rotates the token exactly once.
    let existing = cookie_value(&headers, state.csrf.cookie_name()).filter(|token| state.csrf.verify(token, &subject));
    let rotated = existing.is_none();
    let csrf_token = existing.unwrap_or_else(|| state.csrf.issue(&subject));

    let check = csrf::check(&state.csrf, &method, &headers, &subject);
    let context = RequestCtx {
        surface,
        host,
        base_domain: state.config.base_domain.clone(),
        path: path.clone(),
        nonce: new_nonce(),
        csrf_token: csrf_token.clone(),
        csrf: check,
        actor,
        is_htmx: headers.get("hx-request").is_some(),
        subject,
        release_manifest_url: state.config.release_manifest_url.clone(),
        chat_enabled: state.config.chat_enabled,
    };
    let nonce = context.nonce.clone();

    let mut response = if check == CsrfCheck::Rejected {
        WebError::CsrfRejected.into_response()
    } else {
        request.extensions_mut().insert(context);
        next.run(request).await
    };

    if rotated {
        if let Some(cookie) = state.csrf.set_cookie(&csrf_token) {
            response.headers_mut().append(SET_COOKIE, cookie);
        }
    }
    apply_security_headers(&mut response, &nonce, &path);
    response
}

/// Bearer first (API-ish htmx calls), then the host-scoped cookie session.
async fn resolve_actor(state: &AppState, headers: &HeaderMap, surface: Surface) -> Option<Actor> {
    if !surface.is_authenticated_surface() {
        return None;
    }
    if let Some(token) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(bearer_token)
    {
        match state.auth.verify_bearer(token).await {
            Ok(actor) => return Some(actor),
            Err(error) => {
                tracing::debug!(auth.outcome = %error, "bearer credential rejected");
                return None;
            }
        }
    }
    state
        .sessions
        .read(headers, surface, now_unix())
        .map(|session| Actor::from_session(&session))
}

/// A fresh 128-bit nonce per request. Never derived from anything guessable.
#[must_use]
pub fn new_nonce() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// The Content-Security-Policy for this response.
///
/// `script-src` allows this origin plus the request's nonce and nothing else —
/// no CDN, no inline handler, no `eval`. `connect-src 'self'` covers the
/// same-origin WebSocket relay at `/ws`.
#[must_use]
pub fn content_security_policy(nonce: &str) -> String {
    format!(
        "default-src 'self'; \
script-src 'self' 'nonce-{nonce}'; \
style-src 'self' 'nonce-{nonce}'; \
img-src 'self' data:; \
font-src 'self'; \
connect-src 'self'; \
form-action 'self'; \
frame-ancestors 'none'; \
base-uri 'none'; \
object-src 'none'"
    )
}

fn apply_security_headers(response: &mut Response, nonce: &str, path: &str) {
    let headers = response.headers_mut();
    insert_static(headers, "content-security-policy", &content_security_policy(nonce));
    insert_static(headers, "x-content-type-options", "nosniff");
    insert_static(headers, "referrer-policy", "strict-origin-when-cross-origin");
    insert_static(headers, "x-frame-options", "DENY");
    insert_static(
        headers,
        "permissions-policy",
        "accelerometer=(), camera=(), geolocation=(), gyroscope=(), microphone=(), payment=(), usb=()",
    );
    insert_static(headers, "cross-origin-opener-policy", "same-origin");

    // Content-addressed release assets are immutable; everything else must not
    // be cached by a shared cache, because pages carry a per-user CSRF token.
    let cache = if path.starts_with(crate::ui::loader::RELEASE_PREFIX) {
        "public, max-age=31536000, immutable"
    } else if path.starts_with("/assets/") {
        "public, max-age=3600"
    } else {
        "private, no-store"
    };
    if let Ok(value) = HeaderValue::from_str(cache) {
        headers.insert(CACHE_CONTROL, value);
    }
}

fn insert_static(headers: &mut HeaderMap, name: &'static str, value: &str) {
    if let (Ok(name), Ok(value)) = (HeaderName::from_bytes(name.as_bytes()), HeaderValue::from_str(value)) {
        headers.insert(name, value);
    }
}

fn peer_ip(request: &Request) -> Option<IpAddr> {
    request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|info| info.0.ip())
}

/// Installs ores-middleware with this service's auth and rate-limit hooks.
///
/// `install_from_env_with_ores_logger` is the shortcut for a service with no
/// integrations; this server has both, so it builds the stack from the same env
/// and attaches them before installing. Set
/// `GHA_INDIE_WORKER_MIDDLEWARE_INTEGRATIONS=off` to fall back to the shortcut
/// (useful when bisecting a production incident down to the pipeline itself).
pub fn install(router: Router, state: &AppState, logger: Option<Logger>) -> Result<Router, BootstrapError> {
    let integrations_enabled = std::env::var("GHA_INDIE_WORKER_MIDDLEWARE_INTEGRATIONS")
        .map(|value| !value.eq_ignore_ascii_case("off"))
        .unwrap_or(true);

    if !integrations_enabled {
        return match logger {
            Some(logger) => ores_middleware::frameworks::axum::install_from_env_with_ores_logger(
                router,
                crate::SERVICE_NAME,
                logger,
            ),
            None => ores_middleware::frameworks::axum::install_from_env(router, crate::SERVICE_NAME),
        };
    }

    let verifier = Arc::new(WebAuthVerifier::new(
        Arc::clone(&state.auth),
        Arc::clone(&state.sessions),
        Arc::clone(&state.hosts),
    ));
    let limiter = Arc::new(OpaqueRateLimiter::new(
        state
            .config
            .rate_limit_hmac_secret
            .as_deref()
            .unwrap_or("gha-indie-worker-web-dev-limiter"),
    ));
    let stack = Arc::new(
        ores_middleware::stack_from_env(crate::SERVICE_NAME)?
            .with_auth_verifier(verifier)
            .with_rate_limiter(limiter),
    );
    Ok(match logger {
        Some(logger) => ores_middleware::frameworks::axum::install_with_ores_logger(router, stack, logger),
        None => ores_middleware::frameworks::axum::install(router, stack),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::csrf::CsrfKey;

    fn context(check: CsrfCheck, subject: &str) -> RequestCtx {
        RequestCtx {
            surface: Surface::App,
            host: "app.indiebuild.dev".into(),
            base_domain: "indiebuild.dev".into(),
            path: "/".into(),
            nonce: "n".into(),
            csrf_token: "t".into(),
            csrf: check,
            actor: None,
            is_htmx: false,
            subject: subject.into(),
            release_manifest_url: None,
            chat_enabled: true,
        }
    }

    #[test]
    fn nonces_are_unique_per_request() {
        let a = new_nonce();
        let b = new_nonce();
        assert_ne!(a, b);
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn the_policy_pins_scripts_to_this_origin_and_the_nonce() {
        let policy = content_security_policy("abc123");
        assert!(policy.contains("script-src 'self' 'nonce-abc123'"));
        assert!(policy.contains("style-src 'self' 'nonce-abc123'"));
        assert!(policy.contains("frame-ancestors 'none'"));
        assert!(policy.contains("base-uri 'none'"));
        assert!(!policy.contains("unsafe-inline"));
        assert!(!policy.contains("unsafe-eval"));
        assert!(!policy.contains("http"));
    }

    #[test]
    fn a_deferred_form_token_is_verified_against_the_subject() {
        let key = CsrfKey::new("secret", "giw_csrf", true);
        let token = key.issue("user_1");
        let context = context(CsrfCheck::DeferredToForm, "user_1");
        assert!(context.verify_form_csrf(&key, Some(&token)).is_ok());
        assert!(context.verify_form_csrf(&key, None).is_err());
        assert!(context.verify_form_csrf(&key, Some("nope")).is_err());

        let other = context(CsrfCheck::DeferredToForm, "user_2");
        assert!(other.verify_form_csrf(&key, Some(&token)).is_err());
    }

    #[test]
    fn a_header_verified_request_does_not_re_check_the_form() {
        let key = CsrfKey::new("secret", "giw_csrf", true);
        assert!(context(CsrfCheck::HeaderAccepted, "u")
            .verify_form_csrf(&key, None)
            .is_ok());
        assert!(context(CsrfCheck::NotRequired, "u")
            .verify_form_csrf(&key, None)
            .is_ok());
        assert!(context(CsrfCheck::Rejected, "u")
            .verify_form_csrf(&key, Some("x"))
            .is_err());
    }

    #[test]
    fn release_assets_are_immutable_and_pages_are_never_shared_cached() {
        let mut response = axum::response::Response::new(axum::body::Body::empty());
        apply_security_headers(&mut response, "n", "/assets/releases/abc/app.js");
        assert_eq!(
            response.headers().get(CACHE_CONTROL).and_then(|v| v.to_str().ok()),
            Some("public, max-age=31536000, immutable")
        );

        let mut page = axum::response::Response::new(axum::body::Body::empty());
        apply_security_headers(&mut page, "n", "/runs");
        assert_eq!(
            page.headers().get(CACHE_CONTROL).and_then(|v| v.to_str().ok()),
            Some("private, no-store")
        );
        assert!(page.headers().contains_key("content-security-policy"));
        assert_eq!(
            page.headers().get("x-frame-options").and_then(|v| v.to_str().ok()),
            Some("DENY")
        );
    }
}
