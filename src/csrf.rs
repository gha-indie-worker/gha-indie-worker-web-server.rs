//! Cookies and CSRF — the pure half.
//!
//! Everything in this file is a `std`-only function over `&str`. That is deliberate: the rules
//! that decide whether a state-changing request is honoured must be exhaustively testable without
//! a socket, a clock or a router, and they are compiled standalone in CI:
//!
//! ```text
//! rustc --edition 2021 --test src/csrf.rs -o /tmp/csrf && /tmp/csrf
//! ```
//!
//! The scheme is a **synchronizer token that is also a double-submit cookie**:
//!
//! 1. The server mints a random token and puts it in a `HttpOnly; Secure; SameSite=Lax` cookie.
//! 2. Every state-changing form renders the same token into a hidden input.
//! 3. On POST the server requires the cookie, the field, **and** — once a session exists — the
//!    token recorded server-side for that session, and requires all three to be equal.
//!
//! The cookie is `HttpOnly`, so the field is filled in by the server's own renderer rather than by
//! script reading the cookie. That keeps the double-submit property (a cross-site attacker cannot
//! read the cookie to forge the field) while removing the usual weakness of double-submit — that
//! any subdomain able to *set* a cookie can choose both halves — because `bound` is the value the
//! server itself stored for the session, and a cookie the attacker planted will not match it.

/// Name of the CSRF cookie. Distinct from the session cookie: the session cookie is the
/// credential, this one is only ever compared.
pub const COOKIE_NAME: &str = "giw_csrf";
/// Name of the hidden form field carrying the same token.
pub const FIELD_NAME: &str = "csrf_token";
/// Name of the header HTMX sends it in, for requests with no form body.
pub const HEADER_NAME: &str = "x-csrf-token";
/// Name of the session cookie.
pub const SESSION_COOKIE: &str = "giw_session";

/// Why a state-changing request was refused. Every variant is a refusal; none of them is ever
/// shown to the client in detail, because which half was missing is information an attacker can
/// use to iterate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CsrfError {
    /// No `giw_csrf` cookie, or more than one (cookie tossing).
    MissingCookie,
    /// No `csrf_token` field and no `x-csrf-token` header.
    MissingToken,
    /// Present but not shaped like a token we mint.
    Malformed,
    /// Present and well formed, but not equal to what we expect.
    Mismatch,
}

impl CsrfError {
    /// One message for all of them, for the same reason [`crate::csrf`] refuses to say which
    /// check failed.
    #[must_use]
    pub const fn public_message(self) -> &'static str {
        "That form expired or came from somewhere we do not recognise. Reload the page and try again."
    }
}

/// Tokens are 32 bytes rendered as 64 lowercase hex characters, but the check is deliberately
/// written as a shape test over a character class rather than an exact length, so rotating to a
/// longer token never becomes a silent authentication bypass in the other direction.
#[must_use]
pub fn is_well_formed(token: &str) -> bool {
    let length = token.len();
    (32..=128).contains(&length)
        && token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

/// Compare two tokens without an early exit on the first differing byte.
///
/// Lengths are compared first and that comparison is not constant time; token length is public
/// (it is a constant of the scheme), the token itself is not.
#[must_use]
pub fn constant_time_eq(left: &str, right: &str) -> bool {
    let (left, right) = (left.as_bytes(), right.as_bytes());
    if left.len() != right.len() {
        return false;
    }
    let mut difference: u8 = 0;
    for (a, b) in left.iter().zip(right.iter()) {
        difference |= a ^ b;
    }
    difference == 0
}

/// The whole CSRF decision, as one pure function.
///
/// * `cookie` — the value of the `giw_csrf` cookie, if exactly one was sent.
/// * `submitted` — the `csrf_token` form field, or the `x-csrf-token` header.
/// * `bound` — the token the server recorded for this session, or `None` before sign-in, when
///   there is no session to bind to and the cookie is the only anchor.
///
/// # Errors
/// [`CsrfError`], never distinguishing between them to the client.
pub fn verify(
    cookie: Option<&str>,
    submitted: Option<&str>,
    bound: Option<&str>,
) -> Result<(), CsrfError> {
    let cookie = cookie.ok_or(CsrfError::MissingCookie)?;
    let submitted = submitted.ok_or(CsrfError::MissingToken)?;
    if !is_well_formed(cookie) || !is_well_formed(submitted) {
        return Err(CsrfError::Malformed);
    }
    if !constant_time_eq(cookie, submitted) {
        return Err(CsrfError::Mismatch);
    }
    if let Some(bound) = bound {
        // A signed-in request must also match the token the server itself stored, so a planted
        // cookie plus a matching field — the classic double-submit forgery — still fails.
        if !is_well_formed(bound) || !constant_time_eq(cookie, bound) {
            return Err(CsrfError::Mismatch);
        }
    }
    Ok(())
}

/// Defence in depth alongside `SameSite=Lax`: the `Origin` of a state-changing request must be
/// this exact host over https (or http on a loopback development origin).
///
/// A **missing** `Origin` is refused rather than tolerated: every browser that can run this app
/// sends one on a cross-origin form POST, and "absent means same-origin" is precisely the
/// assumption that makes the check useless.
#[must_use]
pub fn origin_is_ours(origin: Option<&str>, expected_host: &str, require_https: bool) -> bool {
    let Some(origin) = origin else { return false };
    let origin = origin.trim();
    let rest = match origin.strip_prefix("https://") {
        Some(rest) => rest,
        None => match origin.strip_prefix("http://") {
            Some(rest) if !require_https => rest,
            _ => return false,
        },
    };
    // No path, query or userinfo may appear in an Origin; anything else is not one.
    if rest.contains('/') || rest.contains('@') || rest.contains('?') || rest.is_empty() {
        return false;
    }
    rest.eq_ignore_ascii_case(expected_host)
}

// ---------------------------------------------------------------------------------------------
// Cookie parsing
// ---------------------------------------------------------------------------------------------

/// Split a `Cookie:` header into name/value pairs, in the order sent.
///
/// Values are returned exactly as received minus surrounding whitespace and one optional pair of
/// double quotes; nothing is percent-decoded, because every cookie this server sets is drawn from
/// `[A-Za-z0-9_-]` and a value that needs decoding is one we did not write.
#[must_use]
pub fn parse_cookies(header: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    for part in header.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let Some((name, value)) = part.split_once('=') else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        let value = value.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .unwrap_or(value);
        out.push((name, value));
    }
    out
}

/// Every value sent for `name`. More than one is not normal traffic.
#[must_use]
pub fn cookie_values<'a>(header: &'a str, name: &str) -> Vec<&'a str> {
    parse_cookies(header)
        .into_iter()
        .filter(|(candidate, _)| *candidate == name)
        .map(|(_, value)| value)
        .collect()
}

/// The single value sent for `name`.
///
/// Returns `None` when the cookie appears more than once. A duplicate is how "cookie tossing"
/// from a sibling subdomain works: the attacker cannot overwrite our host-scoped cookie, so they
/// add a second one and hope the reader takes theirs. Refusing an ambiguous read costs a signed-in
/// user one reload and costs the attack everything.
#[must_use]
pub fn cookie_value<'a>(header: &'a str, name: &str) -> Option<&'a str> {
    let values = cookie_values(header, name);
    match values.as_slice() {
        [only] => Some(*only),
        _ => None,
    }
}

/// Same, over an optional header (the shape `HeaderMap::get(..).and_then(..)` produces).
#[must_use]
pub fn cookie_from<'a>(header: Option<&'a str>, name: &str) -> Option<&'a str> {
    cookie_value(header?, name)
}

/// How long a cookie lives.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CookieLife {
    /// Until the browser is closed.
    Session,
    /// This many seconds.
    Seconds(i64),
    /// Delete it now.
    Expire,
}

/// Build a `Set-Cookie` value.
///
/// `domain` is the **apex**, not the current host: signing in on `user.` has to be visible on
/// `app.`, so the cookie is deliberately shared across the product's subdomains. That sharing is
/// the reason `admin.` is served by a different binary that never reads this cookie.
#[must_use]
pub fn set_cookie(
    name: &str,
    value: &str,
    domain: Option<&str>,
    life: CookieLife,
    secure: bool,
    http_only: bool,
) -> String {
    let mut out = String::with_capacity(128);
    out.push_str(name);
    out.push('=');
    if !matches!(life, CookieLife::Expire) {
        out.push_str(value);
    }
    out.push_str("; Path=/; SameSite=Lax");
    if http_only {
        out.push_str("; HttpOnly");
    }
    if secure {
        out.push_str("; Secure");
    }
    if let Some(domain) = domain {
        if !domain.is_empty() && domain != "localhost" && !domain.starts_with("127.") {
            out.push_str("; Domain=");
            out.push_str(domain);
        }
    }
    match life {
        CookieLife::Session => {}
        CookieLife::Seconds(seconds) => {
            out.push_str("; Max-Age=");
            out.push_str(&seconds.to_string());
        }
        CookieLife::Expire => out.push_str("; Max-Age=0"),
    }
    out
}

/// Parse an `application/x-www-form-urlencoded` body into pairs, decoding `%XX` and `+`.
///
/// The forms this server renders are small and flat, so this is a list rather than a map: a field
/// sent twice is visible to the caller instead of being silently last-write-wins.
#[must_use]
pub fn parse_form(body: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for part in body.split('&') {
        if part.is_empty() {
            continue;
        }
        let (name, value) = part.split_once('=').unwrap_or((part, ""));
        out.push((percent_decode(name), percent_decode(value)));
    }
    out
}

/// First value for `name` in a parsed form.
#[must_use]
pub fn form_value<'a>(form: &'a [(String, String)], name: &str) -> Option<&'a str> {
    form.iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                match (hex(bytes[index + 1]), hex(bytes[index + 2])) {
                    (Some(high), Some(low)) => {
                        out.push((high << 4) | low);
                        index += 3;
                    }
                    _ => {
                        out.push(b'%');
                        index += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

const fn hex(byte: u8) -> Option<u8> {
    Some(match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        b'A'..=b'F' => byte - b'A' + 10,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "6f1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f9";
    const OTHER: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn a_matching_cookie_field_and_session_token_is_accepted() {
        assert_eq!(verify(Some(GOOD), Some(GOOD), Some(GOOD)), Ok(()));
        // Before sign-in there is no session token to bind to.
        assert_eq!(verify(Some(GOOD), Some(GOOD), None), Ok(()));
    }

    #[test]
    fn every_missing_half_is_refused() {
        assert_eq!(
            verify(None, Some(GOOD), None),
            Err(CsrfError::MissingCookie)
        );
        assert_eq!(verify(Some(GOOD), None, None), Err(CsrfError::MissingToken));
        assert_eq!(verify(None, None, None), Err(CsrfError::MissingCookie));
    }

    #[test]
    fn a_planted_cookie_that_matches_the_field_still_fails_once_signed_in() {
        // The forgery double-submit alone cannot stop: the attacker sets both halves to a value
        // they chose. The server's own record of the session's token is what refuses it.
        assert_eq!(
            verify(Some(OTHER), Some(OTHER), Some(GOOD)),
            Err(CsrfError::Mismatch)
        );
    }

    #[test]
    fn mismatched_halves_are_refused() {
        assert_eq!(
            verify(Some(GOOD), Some(OTHER), Some(GOOD)),
            Err(CsrfError::Mismatch)
        );
    }

    #[test]
    fn junk_is_refused_before_it_is_compared() {
        for bad in [
            "",
            "short",
            "  ",
            &"a".repeat(129),
            "has spaces in it aaaaaaaaaaaaaaaaaaaaaa",
            "tok;en=x00000000000000000000000000000000",
        ] {
            assert_eq!(
                verify(Some(bad), Some(bad), None),
                Err(CsrfError::Malformed),
                "input {bad:?}"
            );
        }
        // A malformed *stored* token can never satisfy the check either.
        assert_eq!(
            verify(Some(GOOD), Some(GOOD), Some("nope")),
            Err(CsrfError::Mismatch)
        );
    }

    #[test]
    fn constant_time_eq_agrees_with_equality() {
        assert!(constant_time_eq(GOOD, GOOD));
        assert!(!constant_time_eq(GOOD, OTHER));
        assert!(!constant_time_eq(GOOD, &GOOD[..GOOD.len() - 1]));
        assert!(constant_time_eq("", ""));
    }

    #[test]
    fn cookies_parse_with_whitespace_quotes_and_junk() {
        let header = " a=1;b = 2 ; ;=orphan; c=\"three\"; novalue; d=has=equals ";
        assert_eq!(
            parse_cookies(header),
            vec![("a", "1"), ("b", "2"), ("c", "three"), ("d", "has=equals")]
        );
        assert_eq!(cookie_value(header, "a"), Some("1"));
        assert_eq!(cookie_value(header, "missing"), None);
        assert_eq!(cookie_value("", "a"), None);
    }

    #[test]
    fn a_duplicated_cookie_reads_as_absent() {
        let tossed = format!("{COOKIE_NAME}={GOOD}; {COOKIE_NAME}={OTHER}");
        assert_eq!(cookie_values(&tossed, COOKIE_NAME), vec![GOOD, OTHER]);
        assert_eq!(cookie_value(&tossed, COOKIE_NAME), None);
        assert_eq!(
            verify(cookie_value(&tossed, COOKIE_NAME), Some(GOOD), Some(GOOD)),
            Err(CsrfError::MissingCookie)
        );
    }

    #[test]
    fn set_cookie_always_carries_the_defensive_attributes() {
        let value = set_cookie(
            SESSION_COOKIE,
            GOOD,
            Some("indiebuild.dev"),
            CookieLife::Seconds(86_400),
            true,
            true,
        );
        assert!(value.starts_with("giw_session="));
        for expected in [
            "; Path=/",
            "; SameSite=Lax",
            "; HttpOnly",
            "; Secure",
            "; Domain=indiebuild.dev",
            "; Max-Age=86400",
        ] {
            assert!(value.contains(expected), "missing {expected} in {value}");
        }
        // Development over loopback must not set a Domain or claim Secure.
        let dev = set_cookie(
            SESSION_COOKIE,
            GOOD,
            Some("localhost"),
            CookieLife::Session,
            false,
            true,
        );
        assert!(!dev.contains("Domain="));
        assert!(!dev.contains("Secure"));
        assert!(!dev.contains("Max-Age"));
        // Expiry clears the value as well as the age.
        let gone = set_cookie(SESSION_COOKIE, GOOD, None, CookieLife::Expire, true, true);
        assert!(gone.starts_with("giw_session=;"));
        assert!(gone.contains("Max-Age=0"));
    }

    #[test]
    fn origins_are_checked_exactly() {
        assert!(origin_is_ours(
            Some("https://org.indiebuild.dev"),
            "org.indiebuild.dev",
            true
        ));
        assert!(origin_is_ours(
            Some("https://ORG.IndieBuild.dev"),
            "org.indiebuild.dev",
            true
        ));
        for bad in [
            "https://org.indiebuild.dev.evil.test",
            "https://evil.test",
            "http://org.indiebuild.dev",
            "https://org.indiebuild.dev/path",
            "https://user:pw@org.indiebuild.dev",
            "null",
            "",
        ] {
            assert!(
                !origin_is_ours(Some(bad), "org.indiebuild.dev", true),
                "origin {bad}"
            );
        }
        assert!(
            !origin_is_ours(None, "org.indiebuild.dev", true),
            "a missing Origin is refused"
        );
        // Loopback development may use http.
        assert!(origin_is_ours(
            Some("http://127.0.0.1:8081"),
            "127.0.0.1:8081",
            false
        ));
    }

    #[test]
    fn forms_decode_percent_escapes_and_plus() {
        let form =
            parse_form("email=alex%40acme.test&role=member&note=a+b%2Bc&empty=&odd%=1&trailing=%2");
        assert_eq!(form_value(&form, "email"), Some("alex@acme.test"));
        assert_eq!(form_value(&form, "role"), Some("member"));
        assert_eq!(form_value(&form, "note"), Some("a b+c"));
        assert_eq!(form_value(&form, "empty"), Some(""));
        assert_eq!(form_value(&form, "missing"), None);
        // A stray percent is kept literally rather than eating the next characters.
        assert_eq!(form_value(&form, "odd%"), Some("1"));
        assert_eq!(form_value(&form, "trailing"), Some("%2"));
    }

    #[test]
    fn a_field_sent_twice_is_visible_rather_than_collapsed() {
        let form = parse_form("csrf_token=a&csrf_token=b");
        assert_eq!(form.len(), 2);
        assert_eq!(form_value(&form, "csrf_token"), Some("a"));
    }
}
