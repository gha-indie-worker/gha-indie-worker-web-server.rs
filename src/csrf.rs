#![forbid(unsafe_code)]

//! Double-submit CSRF tokens.
//!
//! A token is `<nonce>.<hmac(secret, nonce || subject)>`. It is written to a
//! **readable** cookie (htmx must be able to echo it) and to every form, and a
//! state-changing request must present the same value in either the
//! `X-CSRF-Token` header or a `csrf_token` form field.
//!
//! Because the token is bound to the session subject and signed with the server
//! secret, a sibling origin cannot mint one that this surface will accept: it
//! would need the secret. Anonymous visitors get a token bound to the literal
//! subject `anonymous`, which still stops a blind cross-site POST.
//!
//! `SameSite=Lax` on the session cookie is the first line of defence; this is
//! the second, because Lax alone does not cover every legacy client.

use axum::http::{HeaderMap, HeaderValue, Method};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

use crate::session::{from_hex, to_hex};

type HmacSha256 = Hmac<Sha256>;

/// Header htmx sends (see `ui::layout`, which sets `hx-headers` on `<body>`).
pub const CSRF_HEADER: &str = "x-csrf-token";
/// Hidden field name used by no-JavaScript form fallbacks.
pub const CSRF_FIELD: &str = "csrf_token";
/// Subject used before sign-in.
pub const ANONYMOUS_SUBJECT: &str = "anonymous";

#[derive(Clone)]
pub struct CsrfKey {
    secret: Vec<u8>,
    cookie_name: String,
    secure: bool,
}

impl std::fmt::Debug for CsrfKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CsrfKey")
            .field("cookie_name", &self.cookie_name)
            .finish_non_exhaustive()
    }
}

impl CsrfKey {
    #[must_use]
    pub fn new(secret: &str, cookie_name: impl Into<String>, secure: bool) -> Self {
        Self {
            secret: secret.as_bytes().to_vec(),
            cookie_name: cookie_name.into(),
            secure,
        }
    }

    #[must_use]
    pub fn cookie_name(&self) -> &str {
        &self.cookie_name
    }

    fn tag(&self, nonce: &str, subject: &str) -> String {
        let mut mac = <HmacSha256 as Mac>::new_from_slice(&self.secret).expect("hmac accepts any key length");
        mac.update(nonce.as_bytes());
        mac.update(b"\x00");
        mac.update(subject.as_bytes());
        to_hex(&mac.finalize().into_bytes())
    }

    /// Mints a token bound to `subject`.
    #[must_use]
    pub fn issue(&self, subject: &str) -> String {
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let tag = self.tag(&nonce, subject);
        format!("{nonce}.{tag}")
    }

    /// True when `token` was minted by this key for this subject.
    #[must_use]
    pub fn verify(&self, token: &str, subject: &str) -> bool {
        let Some((nonce, tag)) = token.split_once('.') else {
            return false;
        };
        if nonce.len() != 32 || from_hex(nonce).is_none() {
            return false;
        }
        let expected = self.tag(nonce, subject);
        expected.as_bytes().ct_eq(tag.as_bytes()).unwrap_u8() == 1
    }

    /// The readable double-submit cookie. Not `HttpOnly` by design.
    #[must_use]
    pub fn set_cookie(&self, token: &str) -> Option<HeaderValue> {
        let mut cookie = format!("{}={token}; Path=/; SameSite=Lax", self.cookie_name);
        if self.secure {
            cookie.push_str("; Secure");
        }
        HeaderValue::from_str(&cookie).ok()
    }
}

/// Methods that may change state and therefore need a token.
#[must_use]
pub fn is_unsafe_method(method: &Method) -> bool {
    !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS | Method::TRACE)
}

/// The token presented in the request header, if any.
#[must_use]
pub fn header_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get(CSRF_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

/// True when the body may still carry a `csrf_token` field, so the handler —
/// not the middleware — is the place that decides.
#[must_use]
pub fn body_may_carry_token(headers: &HeaderMap) -> bool {
    headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|value| {
            let value = value.to_ascii_lowercase();
            value.starts_with("application/x-www-form-urlencoded") || value.starts_with("multipart/form-data")
        })
}

/// The decision the request-context middleware makes before dispatch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CsrfCheck {
    /// Safe method, nothing to check.
    NotRequired,
    /// A valid header token was presented.
    HeaderAccepted,
    /// No header, but the body is a form — the handler must call [`CsrfKey::verify`].
    DeferredToForm,
    /// Reject with 403.
    Rejected,
}

/// Middleware-level check. Runs before any handler and before any body is read.
#[must_use]
pub fn check(key: &CsrfKey, method: &Method, headers: &HeaderMap, subject: &str) -> CsrfCheck {
    if !is_unsafe_method(method) {
        return CsrfCheck::NotRequired;
    }
    match header_token(headers) {
        Some(token) if key.verify(&token, subject) => CsrfCheck::HeaderAccepted,
        Some(_) => CsrfCheck::Rejected,
        None if body_may_carry_token(headers) => CsrfCheck::DeferredToForm,
        None => CsrfCheck::Rejected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> CsrfKey {
        CsrfKey::new("csrf-unit-secret", "giw_csrf", true)
    }

    fn form_headers(token: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::CONTENT_TYPE,
            HeaderValue::from_static("application/x-www-form-urlencoded"),
        );
        if let Some(token) = token {
            headers.insert(CSRF_HEADER, HeaderValue::from_str(token).expect("ascii token"));
        }
        headers
    }

    #[test]
    fn a_minted_token_verifies_for_its_subject_only() {
        let key = key();
        let token = key.issue("user_1");
        assert!(key.verify(&token, "user_1"));
        assert!(!key.verify(&token, "user_2"));
    }

    #[test]
    fn a_token_from_another_secret_is_rejected() {
        let token = key().issue("user_1");
        assert!(!CsrfKey::new("other", "giw_csrf", true).verify(&token, "user_1"));
    }

    #[test]
    fn malformed_tokens_are_rejected_without_panicking() {
        let key = key();
        for token in [
            "",
            ".",
            "abc",
            "abc.def",
            &"0".repeat(32),
            &format!("{}.zz", "0".repeat(32)),
        ] {
            assert!(!key.verify(token, "user_1"), "accepted {token:?}");
        }
    }

    #[test]
    fn safe_methods_need_no_token() {
        assert_eq!(
            check(&key(), &Method::GET, &HeaderMap::new(), "anonymous"),
            CsrfCheck::NotRequired
        );
        assert_eq!(
            check(&key(), &Method::HEAD, &HeaderMap::new(), "anonymous"),
            CsrfCheck::NotRequired
        );
    }

    #[test]
    fn a_post_without_any_token_is_rejected() {
        assert_eq!(
            check(&key(), &Method::POST, &HeaderMap::new(), "anonymous"),
            CsrfCheck::Rejected
        );
    }

    #[test]
    fn a_post_with_a_valid_header_is_accepted() {
        let key = key();
        let token = key.issue("user_1");
        let headers = form_headers(Some(&token));
        assert_eq!(
            check(&key, &Method::POST, &headers, "user_1"),
            CsrfCheck::HeaderAccepted
        );
    }

    #[test]
    fn a_post_with_a_forged_header_is_rejected_even_for_a_form() {
        let key = key();
        let headers = form_headers(Some("deadbeef.deadbeef"));
        assert_eq!(check(&key, &Method::POST, &headers, "user_1"), CsrfCheck::Rejected);
    }

    #[test]
    fn a_form_post_without_a_header_defers_to_the_handler() {
        assert_eq!(
            check(&key(), &Method::POST, &form_headers(None), "user_1"),
            CsrfCheck::DeferredToForm
        );
    }

    #[test]
    fn the_cookie_is_readable_by_script_on_purpose() {
        let cookie = key().set_cookie("token").expect("header value");
        let text = cookie.to_str().expect("ascii");
        assert!(!text.contains("HttpOnly"));
        assert!(text.contains("SameSite=Lax"));
        assert!(!text.to_ascii_lowercase().contains("domain="));
    }
}
