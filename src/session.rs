#![forbid(unsafe_code)]

//! Cookie sessions for the browser surfaces.
//!
//! ## Why the cookie is host-scoped and not domain-wide
//!
//! `app.`, `user.`, `org.` and `m.` are **separate origins**. A cookie written
//! with `Domain=indiebuild.dev` would be sent to every one of them — and to any
//! future subdomain, including anything a customer ever gets to control. The
//! org surface carries seat, billing and member-role authority, so a session
//! minted on `user.` must not be replayable there.
//!
//! Therefore no `Domain` attribute is ever emitted. A cookie set on
//! `org.indiebuild.dev` is returned only to `org.indiebuild.dev`. Signing in on
//! two surfaces means completing the shared-auth exchange twice, once per
//! origin, which is exactly the boundary we want.
//!
//! Attributes: `HttpOnly` (script can never read the token), `SameSite=Lax`
//! (top-level navigations from the marketing home still arrive signed in, while
//! cross-site POSTs do not), `Secure` outside development, `Path=/`.

use std::fmt::Write as _;

use axum::extract::FromRequestParts;
use axum::http::header::{COOKIE, SET_COOKIE};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Redirect, Response};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use subtle::ConstantTimeEq;

use crate::hosts::Surface;

type HmacSha256 = Hmac<Sha256>;

/// Signed-cookie payload. Small on purpose: identity plus the delegated
/// shared-auth access token, nothing a page could not re-fetch.
///
/// `Debug` is implemented by hand: this type holds a bearer token, and a
/// derived `Debug` would put it in the first log line that ever formats a
/// request context.
#[derive(Clone, Deserialize, Serialize)]
pub struct SessionData {
    pub sub: String,
    pub access_token: String,
    /// Unix seconds. Checked on every decode; an expired cookie is not a session.
    pub expires_at: i64,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub org_id: Option<String>,
    #[serde(default)]
    pub roles: Vec<String>,
    #[serde(default)]
    pub scopes: Vec<String>,
    /// Authentication context class from shared-auth (`aal2` after step-up).
    #[serde(default)]
    pub acr: Option<String>,
    /// The surface this session was minted on. A cookie replayed on another
    /// surface is rejected even if an attacker managed to move it.
    pub surface: String,
}

impl std::fmt::Debug for SessionData {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SessionData")
            .field("sub", &self.sub)
            .field("access_token", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .field("org_id", &self.org_id)
            .field("roles", &self.roles)
            .field("acr", &self.acr)
            .field("surface", &self.surface)
            .finish_non_exhaustive()
    }
}

impl SessionData {
    #[must_use]
    pub fn is_expired(&self, now: i64) -> bool {
        self.expires_at <= now
    }

    #[must_use]
    pub fn has_role(&self, role: &str) -> bool {
        self.roles.iter().any(|candidate| candidate == role)
    }

    #[must_use]
    pub fn stepped_up(&self) -> bool {
        self.acr.as_deref() == Some("aal2") || self.acr.as_deref() == Some("urn:shared-auth:acr:2fa")
    }
}

/// The signed-in principal, as every handler sees it.
///
/// `Debug` redacts `access_token` for the same reason [`SessionData`] does:
/// this value ends up inside `RequestCtx`, which is the thing a future
/// `tracing::debug!` is most likely to format wholesale.
#[derive(Clone)]
pub struct Actor {
    pub sub: String,
    pub email: Option<String>,
    pub org_id: Option<String>,
    pub roles: Vec<String>,
    pub scopes: Vec<String>,
    pub acr: Option<String>,
    pub source: ActorSource,
    /// Forwarded verbatim as `Authorization: Bearer …` on api-server writes.
    pub access_token: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActorSource {
    /// Cookie session minted by the shared-auth code exchange.
    Session,
    /// `Authorization: Bearer …` introspected by shared-auth.
    SharedAuthBearer,
    /// Supabase or Neon session JWT verified locally against JWKS.
    FederatedJwt,
}

impl std::fmt::Debug for Actor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Actor")
            .field("sub", &self.sub)
            .field("org_id", &self.org_id)
            .field("roles", &self.roles)
            .field("acr", &self.acr)
            .field("source", &self.source)
            .field("access_token", &"<redacted>")
            .finish_non_exhaustive()
    }
}

impl Actor {
    #[must_use]
    pub fn from_session(session: &SessionData) -> Self {
        Self {
            sub: session.sub.clone(),
            email: session.email.clone(),
            org_id: session.org_id.clone(),
            roles: session.roles.clone(),
            scopes: session.scopes.clone(),
            acr: session.acr.clone(),
            source: ActorSource::Session,
            access_token: session.access_token.clone(),
        }
    }

    #[must_use]
    pub fn has_role(&self, role: &str) -> bool {
        self.roles.iter().any(|candidate| candidate == role)
    }

    #[must_use]
    pub fn display_name(&self) -> &str {
        self.email.as_deref().unwrap_or(&self.sub)
    }

    #[must_use]
    pub fn stepped_up(&self) -> bool {
        matches!(self.acr.as_deref(), Some("aal2" | "urn:shared-auth:acr:2fa"))
    }
}

/// Signs and verifies session cookies. `v1.<hex(json)>.<hex(hmac)>`.
///
/// Hex rather than base64 keeps the crate free of another dependency and keeps
/// the value in the cookie-safe character set with no percent-encoding.
#[derive(Clone)]
pub struct SessionCodec {
    secret: Vec<u8>,
    cookie_name: String,
    secure: bool,
    ttl_seconds: u64,
}

impl std::fmt::Debug for SessionCodec {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SessionCodec")
            .field("cookie_name", &self.cookie_name)
            .field("secure", &self.secure)
            .field("ttl_seconds", &self.ttl_seconds)
            .finish_non_exhaustive()
    }
}

impl SessionCodec {
    #[must_use]
    pub fn new(secret: &str, cookie_name: impl Into<String>, secure: bool, ttl_seconds: u64) -> Self {
        Self {
            secret: secret.as_bytes().to_vec(),
            cookie_name: cookie_name.into(),
            secure,
            ttl_seconds,
        }
    }

    #[must_use]
    pub fn cookie_name(&self) -> &str {
        &self.cookie_name
    }

    #[must_use]
    pub const fn ttl_seconds(&self) -> u64 {
        self.ttl_seconds
    }

    fn sign(&self, payload: &str) -> String {
        let mut mac = <HmacSha256 as Mac>::new_from_slice(&self.secret).expect("hmac accepts any key length");
        mac.update(payload.as_bytes());
        to_hex(&mac.finalize().into_bytes())
    }

    /// Encodes and signs. Returns `None` only if the payload cannot serialize.
    #[must_use]
    pub fn encode(&self, data: &SessionData) -> Option<String> {
        let json = serde_json::to_vec(data).ok()?;
        let payload = to_hex(&json);
        let signature = self.sign(&payload);
        Some(format!("v1.{payload}.{signature}"))
    }

    /// Verifies the signature in constant time, then the expiry, then the surface.
    #[must_use]
    pub fn decode(&self, raw: &str, surface: Surface, now: i64) -> Option<SessionData> {
        let mut parts = raw.splitn(3, '.');
        if parts.next()? != "v1" {
            return None;
        }
        let payload = parts.next()?;
        let signature = parts.next()?;
        let expected = self.sign(payload);
        if expected.as_bytes().ct_eq(signature.as_bytes()).unwrap_u8() != 1 {
            return None;
        }
        let json = from_hex(payload)?;
        let data: SessionData = serde_json::from_slice(&json).ok()?;
        if data.is_expired(now) {
            return None;
        }
        if data.surface != surface.label() {
            return None;
        }
        Some(data)
    }

    /// `Set-Cookie` for a fresh session. No `Domain` — see the module docs.
    #[must_use]
    pub fn set_cookie(&self, value: &str) -> Option<HeaderValue> {
        let mut cookie = format!(
            "{}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}",
            self.cookie_name, self.ttl_seconds
        );
        if self.secure {
            cookie.push_str("; Secure");
        }
        HeaderValue::from_str(&cookie).ok()
    }

    /// `Set-Cookie` that removes the session.
    #[must_use]
    pub fn clear_cookie(&self) -> Option<HeaderValue> {
        let mut cookie = format!("{}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0", self.cookie_name);
        if self.secure {
            cookie.push_str("; Secure");
        }
        HeaderValue::from_str(&cookie).ok()
    }

    /// Reads and verifies the session cookie out of a request's headers.
    #[must_use]
    pub fn read(&self, headers: &HeaderMap, surface: Surface, now: i64) -> Option<SessionData> {
        let raw = cookie_value(headers, &self.cookie_name)?;
        self.decode(&raw, surface, now)
    }
}

/// Extracts one cookie value from the `Cookie` header(s).
#[must_use]
pub fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    for header in headers.get_all(COOKIE) {
        let Ok(text) = header.to_str() else { continue };
        for pair in text.split(';') {
            let pair = pair.trim();
            if let Some((key, value)) = pair.split_once('=') {
                if key.trim() == name {
                    return Some(value.trim().to_owned());
                }
            }
        }
    }
    None
}

/// Current wall-clock unix seconds.
#[must_use]
pub fn now_unix() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

#[must_use]
pub(crate) fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[must_use]
pub(crate) fn from_hex(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 {
        return None;
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(text.len() / 2);
    let mut index = 0;
    while index < bytes.len() {
        let high = (bytes[index] as char).to_digit(16)?;
        let low = (bytes[index + 1] as char).to_digit(16)?;
        out.push(u8::try_from(high * 16 + low).ok()?);
        index += 2;
    }
    Some(out)
}

/// Where an unauthenticated visitor on `surface` should be sent, preserving the
/// page they wanted. `next` is only ever a path on the same origin.
#[must_use]
pub fn login_redirect(surface: Surface, base_domain: &str, next: Option<&str>) -> Response {
    let origin = surface.login_origin(base_domain);
    let target = match next.filter(|value| is_safe_next(value)) {
        Some(next) => format!("{origin}/login?next={}", percent_encode_path(next)),
        None => format!("{origin}/login"),
    };
    Redirect::to(&target).into_response()
}

/// A `next` value must be a same-origin absolute path. `//evil.com` and
/// `https://evil.com` are open redirects and are rejected.
#[must_use]
pub fn is_safe_next(value: &str) -> bool {
    value.starts_with('/') && !value.starts_with("//") && !value.contains('\\') && !value.contains("://")
}

/// Minimal percent-encoding for a path put in a query string.
#[must_use]
pub fn percent_encode_path(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => out.push(byte as char),
            other => {
                let _ = write!(out, "%{other:02X}");
            }
        }
    }
    out
}

/// Attaches a `Set-Cookie` header to an existing response.
pub fn with_cookie(mut response: Response, cookie: Option<HeaderValue>) -> Response {
    if let Some(cookie) = cookie {
        response.headers_mut().append(SET_COOKIE, cookie);
    }
    response
}

/// Completes the shared-auth code exchange and mints this origin's session.
///
/// The `code` the identity provider hands back is exchanged for a shared-auth
/// access token, which is then verified through the same path as any other
/// bearer — so an admin-instance credential is rejected here too, before it can
/// ever become a cookie.
///
/// The cookie is written with **no `Domain` attribute**: signing in on `org.`
/// does not sign anyone in on `user.` or `app.`.
pub async fn complete_exchange(
    state: &crate::state::AppState,
    context: &crate::middleware::RequestCtx,
    code: Option<&str>,
    next: Option<&str>,
) -> Response {
    let target = next.filter(|value| is_safe_next(value)).unwrap_or("/").to_owned();

    let Some(code) = code.map(str::trim).filter(|code| !code.is_empty()) else {
        return login_redirect(context.surface, &context.base_domain, Some(&target));
    };
    let Some(client) = state.auth.shared_auth() else {
        return crate::WebError::Unavailable.into_response();
    };
    let Ok(exchange) = client.exchange(code).await else {
        return crate::WebError::Unauthenticated.into_response();
    };

    // Verify the freshly minted token exactly as a bearer would be verified, so
    // the roles and organization on the cookie are the ones shared-auth asserts.
    let actor = match state.auth.verify_bearer(&exchange.access_token).await {
        Ok(actor) => actor,
        Err(error) => return crate::WebError::from(error).into_response(),
    };

    let ttl = i64::try_from(state.sessions.ttl_seconds()).unwrap_or(i64::MAX);
    let provider_expiry = i64::try_from(exchange.expires_at).unwrap_or(i64::MAX);
    let session = SessionData {
        sub: if actor.sub.is_empty() {
            exchange.shared_user_id.clone()
        } else {
            actor.sub.clone()
        },
        access_token: exchange.access_token.clone(),
        // Never outlive what the provider granted.
        expires_at: provider_expiry.min(now_unix().saturating_add(ttl)),
        email: actor.email.clone(),
        org_id: actor.org_id.clone(),
        roles: actor.roles.clone(),
        scopes: actor.scopes.clone(),
        acr: actor.acr.clone(),
        surface: context.surface.label().to_owned(),
    };
    let Some(encoded) = state.sessions.encode(&session) else {
        return crate::WebError::Internal.into_response();
    };
    with_cookie(
        Redirect::to(&target).into_response(),
        state.sessions.set_cookie(&encoded),
    )
}

/// Extractor for handlers that require a signed-in principal.
///
/// The principal itself is resolved once per request in
/// [`crate::middleware::request_context`]; this extractor only reads it back, so
/// no handler can accidentally re-run verification with different rules.
impl<S> FromRequestParts<S> for Actor
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let context = parts.extensions.get::<crate::middleware::RequestCtx>().cloned();
        match context {
            Some(context) => match context.actor.clone() {
                Some(actor) => Ok(actor),
                None => Err(login_redirect(
                    context.surface,
                    &context.base_domain,
                    Some(&context.path),
                )),
            },
            None => Err(crate::WebError::Unauthenticated.into_response()),
        }
    }
}

/// Extractor for pages that render differently when signed in but never demand it.
#[derive(Clone, Debug)]
pub struct MaybeActor(pub Option<Actor>);

impl<S> FromRequestParts<S> for MaybeActor
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self(
            parts
                .extensions
                .get::<crate::middleware::RequestCtx>()
                .and_then(|context| context.actor.clone()),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codec() -> SessionCodec {
        SessionCodec::new("unit-test-secret", "giw_session", true, 3_600)
    }

    fn session(surface: Surface, expires_at: i64) -> SessionData {
        SessionData {
            sub: "user_123".into(),
            access_token: "token-abc".into(),
            expires_at,
            email: Some("dev@example.com".into()),
            org_id: None,
            roles: vec!["member".into()],
            scopes: vec!["runs:read".into()],
            acr: None,
            surface: surface.label().to_owned(),
        }
    }

    #[test]
    fn round_trips_a_session() {
        let codec = codec();
        let encoded = codec.encode(&session(Surface::User, 4_000)).expect("encodes");
        let decoded = codec.decode(&encoded, Surface::User, 1_000).expect("decodes");
        assert_eq!(decoded.sub, "user_123");
        assert_eq!(decoded.access_token, "token-abc");
    }

    #[test]
    fn a_tampered_payload_is_rejected() {
        let codec = codec();
        let encoded = codec.encode(&session(Surface::User, 4_000)).expect("encodes");
        let mut parts: Vec<&str> = encoded.split('.').collect();
        let forged = format!("{}ff", parts[1]);
        parts[1] = &forged;
        assert!(codec.decode(&parts.join("."), Surface::User, 1_000).is_none());
    }

    #[test]
    fn a_session_from_another_surface_is_rejected() {
        let codec = codec();
        let encoded = codec.encode(&session(Surface::User, 4_000)).expect("encodes");
        assert!(codec.decode(&encoded, Surface::Org, 1_000).is_none());
        assert!(codec.decode(&encoded, Surface::User, 1_000).is_some());
    }

    #[test]
    fn an_expired_session_is_rejected() {
        let codec = codec();
        let encoded = codec.encode(&session(Surface::User, 1_000)).expect("encodes");
        assert!(codec.decode(&encoded, Surface::User, 1_000).is_none());
        assert!(codec.decode(&encoded, Surface::User, 999).is_some());
    }

    #[test]
    fn a_session_signed_with_another_secret_is_rejected() {
        let encoded = codec().encode(&session(Surface::User, 4_000)).expect("encodes");
        let other = SessionCodec::new("different-secret", "giw_session", true, 3_600);
        assert!(other.decode(&encoded, Surface::User, 1_000).is_none());
    }

    #[test]
    fn the_cookie_is_host_scoped_and_hardened() {
        let cookie = codec().set_cookie("value").expect("header value");
        let text = cookie.to_str().expect("ascii");
        assert!(text.contains("HttpOnly"));
        assert!(text.contains("SameSite=Lax"));
        assert!(text.contains("Secure"));
        assert!(text.contains("Path=/"));
        assert!(
            !text.to_ascii_lowercase().contains("domain="),
            "cookies must not be domain-wide: {text}"
        );
    }

    #[test]
    fn clearing_expires_the_cookie_immediately() {
        let cookie = codec().clear_cookie().expect("header value");
        assert!(cookie.to_str().expect("ascii").contains("Max-Age=0"));
    }

    #[test]
    fn cookie_values_are_read_from_a_multi_pair_header() {
        let mut headers = HeaderMap::new();
        headers.insert(COOKIE, HeaderValue::from_static("other=1; giw_session=abc; giw_csrf=z"));
        assert_eq!(cookie_value(&headers, "giw_session").as_deref(), Some("abc"));
        assert_eq!(cookie_value(&headers, "missing"), None);
    }

    #[test]
    fn a_debug_line_never_prints_a_token() {
        let session = session(Surface::User, 4_000);
        let text = format!("{session:?}");
        assert!(!text.contains("token-abc"));
        assert!(text.contains("<redacted>"));

        let actor = Actor::from_session(&session);
        let text = format!("{actor:?}");
        assert!(!text.contains("token-abc"));
        assert!(text.contains("<redacted>"));
    }

    #[test]
    fn hex_round_trips() {
        assert_eq!(to_hex(&[0x00, 0x0f, 0xff]), "000fff");
        assert_eq!(from_hex("000fff"), Some(vec![0x00, 0x0f, 0xff]));
        assert_eq!(from_hex("zz"), None);
        assert_eq!(from_hex("abc"), None);
    }

    #[test]
    fn open_redirects_are_refused() {
        assert!(is_safe_next("/runs/42"));
        assert!(!is_safe_next("//evil.com"));
        assert!(!is_safe_next("https://evil.com"));
        assert!(!is_safe_next("runs"));
        assert!(!is_safe_next("/a\\b"));
    }
}
