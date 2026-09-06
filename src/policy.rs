//! Response security policy — the pure half.
//!
//! Every header that hardens a response is built here, from values, by functions that import
//! nothing. The router does no more than attach what these return, which means the policy can be
//! read in one place and asserted in tests that do not need a request:
//!
//! ```text
//! rustc --edition 2021 --test src/policy.rs -o /tmp/policy && /tmp/policy
//! ```
//!
//! Two decisions are worth stating rather than leaving to be inferred from the string.
//!
//! **`default-src 'none'`, not `'self'`.** Every fetch directive this app actually uses is
//! written out. The cost is that adding a new kind of subresource means editing this file; the
//! benefit is that adding one *by accident* — a tracking pixel, a font from a CDN, an iframe —
//! is refused by the browser instead of silently working.
//!
//! **Nonces, and no `'unsafe-inline'` anywhere.** The page carries exactly one inline script (the
//! HTMX bootstrap) and no inline styles. Both the nonce and `'self'` appear in `script-src`
//! because we do not use `'strict-dynamic'`: htmx is a plain same-origin `<script src>` with an
//! `integrity` attribute, and `'strict-dynamic'` would make that attribute the only thing
//! standing between us and any script htmx itself chose to inject. A malformed nonce degrades to
//! *no* nonce source, so a bug in nonce generation blocks the inline script rather than opening
//! the policy.

/// A CSP nonce is base64url from our own generator. Anything else is refused before it can be
/// interpolated: a nonce containing `'` or `;` would let a caller write new directives.
#[must_use]
pub fn nonce_is_well_formed(nonce: &str) -> bool {
    (16..=64).contains(&nonce.len())
        && nonce
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

/// What kind of page a policy is being built for. The marketing surface loads no scripts and
/// opens no socket, so it gets a policy that says so.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyKind {
    /// `www.` / apex: static prose, one stylesheet, no script, no socket.
    Marketing,
    /// `user.` / `org.`: forms and HTMX, but no live log tail.
    Interactive,
    /// `app.` / `m.`: HTMX plus the `/ws` log tail.
    Application,
}

impl PolicyKind {
    const fn needs_websocket(self) -> bool {
        matches!(self, PolicyKind::Application)
    }

    const fn needs_script(self) -> bool {
        !matches!(self, PolicyKind::Marketing)
    }
}

/// Build the `Content-Security-Policy` value.
///
/// * `nonce` — the per-response nonce; a malformed one is dropped rather than interpolated.
/// * `host` — the request's host, used to name the WebSocket origin exactly rather than with a
///   wildcard.
/// * `secure` — whether the public origin is https. Off only on a developer's loopback, where the
///   socket is `ws://` and `upgrade-insecure-requests` would break the page.
#[must_use]
pub fn content_security_policy(kind: PolicyKind, nonce: &str, host: &str, secure: bool) -> String {
    let nonce_source = if nonce_is_well_formed(nonce) {
        format!(" 'nonce-{nonce}'")
    } else {
        String::new()
    };

    let mut policy = String::with_capacity(512);
    policy.push_str("default-src 'none'");
    policy.push_str("; base-uri 'none'");
    policy.push_str("; form-action 'self'");
    policy.push_str("; frame-ancestors 'none'");
    policy.push_str("; object-src 'none'");
    policy.push_str("; img-src 'self' data:");
    policy.push_str("; font-src 'self'");
    policy.push_str("; manifest-src 'self'");

    policy.push_str("; style-src 'self'");
    if kind.needs_script() {
        policy.push_str("; script-src 'self'");
        policy.push_str(&nonce_source);
    } else {
        policy.push_str("; script-src 'none'");
    }

    policy.push_str("; connect-src 'self'");
    if kind.needs_websocket() {
        if let Some(origin) = websocket_origin(host, secure) {
            policy.push(' ');
            policy.push_str(&origin);
        }
    }

    if secure {
        policy.push_str("; upgrade-insecure-requests");
    }
    policy
}

/// `app.indiebuild.dev` → `wss://app.indiebuild.dev`. `None` for a host we would not put in a
/// header: a host header is attacker-controlled until it has been matched against the apex, and
/// this is the second line of defence behind that match.
#[must_use]
pub fn websocket_origin(host: &str, secure: bool) -> Option<String> {
    let host = host.trim();
    if host.is_empty() || host.len() > 253 {
        return None;
    }
    let plausible = host
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b':'));
    if !plausible {
        return None;
    }
    Some(format!("{}://{host}", if secure { "wss" } else { "ws" }))
}

/// A header to set on a response, as a name/value pair.
pub type Header = (&'static str, String);

/// Every hardening header, in one list.
///
/// `Strict-Transport-Security` is emitted only over https. A browser ignores it on a cleartext
/// response, so sending it there is not dangerous, only misleading — and the header being absent
/// from a plain-http development response is a useful signal that the process knows it is not in
/// production.
#[must_use]
pub fn security_headers(kind: PolicyKind, nonce: &str, host: &str, secure: bool) -> Vec<Header> {
    let mut headers: Vec<Header> = vec![
        (
            "content-security-policy",
            content_security_policy(kind, nonce, host, secure),
        ),
        ("x-content-type-options", "nosniff".to_owned()),
        (
            "referrer-policy",
            "strict-origin-when-cross-origin".to_owned(),
        ),
        ("x-frame-options", "DENY".to_owned()),
        (
            "permissions-policy",
            // Everything this product could be asked for and never needs. An empty allow-list is
            // the only way to say "not even in an iframe I embedded myself".
            "accelerometer=(), autoplay=(), camera=(), display-capture=(), encrypted-media=(), \
             fullscreen=(), geolocation=(), gyroscope=(), magnetometer=(), microphone=(), \
             midi=(), payment=(), publickey-credentials-get=(), screen-wake-lock=(), usb=(), \
             xr-spatial-tracking=()"
                .to_owned(),
        ),
        ("cross-origin-opener-policy", "same-origin".to_owned()),
        ("cross-origin-resource-policy", "same-origin".to_owned()),
        ("x-permitted-cross-domain-policies", "none".to_owned()),
    ];
    if secure {
        headers.push((
            "strict-transport-security",
            "max-age=63072000; includeSubDomains; preload".to_owned(),
        ));
    }
    headers
}

/// `Cache-Control` for a page that may contain identity. Never a cache entry, anywhere.
pub const PRIVATE_CACHE_CONTROL: &str = "no-store";

/// `Cache-Control` for a fingerprint-free static asset. Short, revalidated, and never `immutable`:
/// the URLs here are not content-addressed, so a long life would pin a stale stylesheet.
pub const ASSET_CACHE_CONTROL: &str = "public, max-age=300, must-revalidate";

#[cfg(test)]
mod tests {
    use super::*;

    const NONCE: &str = "Qk9tZ0hxN3ZfM2ExYjJjM2Q";

    fn directives(policy: &str) -> Vec<&str> {
        policy
            .split(';')
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .collect()
    }

    #[test]
    fn no_policy_ever_contains_an_unsafe_source() {
        for kind in [
            PolicyKind::Marketing,
            PolicyKind::Interactive,
            PolicyKind::Application,
        ] {
            for secure in [true, false] {
                let policy = content_security_policy(kind, NONCE, "app.indiebuild.dev", secure);
                for forbidden in ["'unsafe-inline'", "'unsafe-eval'", "'unsafe-hashes'", "*"] {
                    assert!(
                        !policy.contains(forbidden),
                        "{kind:?}/{secure} admits {forbidden}: {policy}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_nonce_appears_only_where_a_script_is_expected() {
        let application =
            content_security_policy(PolicyKind::Application, NONCE, "app.indiebuild.dev", true);
        assert!(directives(&application)
            .contains(&format!("script-src 'self' 'nonce-{NONCE}'").as_str()));
        // Styles are one external file; there is no inline style to nonce.
        assert!(directives(&application).contains(&"style-src 'self'"));

        let marketing =
            content_security_policy(PolicyKind::Marketing, NONCE, "indiebuild.dev", true);
        assert!(directives(&marketing).contains(&"script-src 'none'"));
        assert!(!marketing.contains(NONCE));
    }

    #[test]
    fn a_malformed_nonce_closes_the_policy_rather_than_opening_it() {
        for bad in [
            "",
            "short",
            "abc'; script-src *; x='",
            "has spaces in it here",
            &"a".repeat(65),
        ] {
            let policy = content_security_policy(PolicyKind::Application, bad, "app.test", true);
            assert!(
                directives(&policy).contains(&"script-src 'self'"),
                "nonce {bad:?}"
            );
            assert!(
                !policy.contains("nonce-"),
                "nonce {bad:?} was interpolated: {policy}"
            );
            assert!(
                !policy.contains("script-src *"),
                "nonce {bad:?} injected a directive"
            );
        }
        assert!(nonce_is_well_formed(NONCE));
        assert!(nonce_is_well_formed("abcdefghijklmnop"));
        assert!(!nonce_is_well_formed("abcdefghijklmno"));
    }

    #[test]
    fn only_the_application_surface_may_open_a_socket_and_only_to_its_own_host() {
        let app =
            content_security_policy(PolicyKind::Application, NONCE, "app.indiebuild.dev", true);
        assert!(directives(&app).contains(&"connect-src 'self' wss://app.indiebuild.dev"));

        let interactive =
            content_security_policy(PolicyKind::Interactive, NONCE, "org.indiebuild.dev", true);
        assert!(directives(&interactive).contains(&"connect-src 'self'"));
        assert!(!interactive.contains("wss://"));

        // A developer on loopback gets ws://, and no upgrade-insecure-requests to break it.
        let local =
            content_security_policy(PolicyKind::Application, NONCE, "127.0.0.1:8081", false);
        assert!(directives(&local).contains(&"connect-src 'self' ws://127.0.0.1:8081"));
        assert!(!local.contains("upgrade-insecure-requests"));
    }

    #[test]
    fn a_hostile_host_header_never_reaches_the_policy() {
        for bad in [
            "app.indiebuild.dev' 'unsafe-inline",
            "app.indiebuild.dev; script-src *",
            "app.indiebuild.dev/path",
            "",
            "   ",
            &"a".repeat(254),
        ] {
            assert_eq!(websocket_origin(bad, true), None, "host {bad:?}");
            let policy = content_security_policy(PolicyKind::Application, NONCE, bad, true);
            assert!(
                directives(&policy).contains(&"connect-src 'self'"),
                "host {bad:?}"
            );
        }
    }

    #[test]
    fn default_src_is_none_and_every_used_directive_is_declared() {
        let policy = content_security_policy(PolicyKind::Application, NONCE, "app.test", true);
        let found = directives(&policy);
        assert!(found.contains(&"default-src 'none'"));
        for directive in [
            "base-uri",
            "form-action",
            "frame-ancestors",
            "object-src",
            "img-src",
            "font-src",
            "manifest-src",
            "style-src",
            "script-src",
            "connect-src",
        ] {
            assert!(
                found.iter().any(|d| d.starts_with(directive)),
                "{directive} is not declared and default-src is 'none'"
            );
        }
        assert!(found.contains(&"frame-ancestors 'none'"));
    }

    #[test]
    fn the_header_set_is_complete_lowercase_and_free_of_duplicates() {
        let headers = security_headers(PolicyKind::Application, NONCE, "app.indiebuild.dev", true);
        let names: Vec<&str> = headers.iter().map(|(name, _)| *name).collect();
        for required in [
            "content-security-policy",
            "x-content-type-options",
            "referrer-policy",
            "x-frame-options",
            "permissions-policy",
            "strict-transport-security",
        ] {
            assert!(names.contains(&required), "{required} is missing");
        }
        for (name, value) in &headers {
            assert_eq!(*name, name.to_ascii_lowercase(), "{name} is not lowercase");
            assert!(!value.is_empty(), "{name} has an empty value");
            assert!(
                !value.contains('\n') && !value.contains('\r'),
                "{name} could split the response"
            );
        }
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "a header is set twice");
    }

    #[test]
    fn hsts_is_only_claimed_over_https() {
        let secure = security_headers(PolicyKind::Application, NONCE, "app.indiebuild.dev", true);
        let plain = security_headers(PolicyKind::Application, NONCE, "127.0.0.1:8081", false);
        assert!(secure
            .iter()
            .any(|(name, _)| *name == "strict-transport-security"));
        assert!(!plain
            .iter()
            .any(|(name, _)| *name == "strict-transport-security"));
        let hsts = secure
            .iter()
            .find(|(name, _)| *name == "strict-transport-security")
            .map(|(_, value)| value.as_str())
            .unwrap();
        assert!(
            hsts.contains("includeSubDomains"),
            "subdomains carry the product surfaces"
        );
        assert!(
            hsts.starts_with("max-age=6"),
            "a short max-age is not a commitment: {hsts}"
        );
    }

    #[test]
    fn permissions_policy_denies_every_capability_it_names() {
        let headers = security_headers(PolicyKind::Marketing, NONCE, "indiebuild.dev", true);
        let value = headers
            .iter()
            .find(|(name, _)| *name == "permissions-policy")
            .map(|(_, value)| value.as_str())
            .unwrap();
        for capability in value.split(',').map(str::trim).filter(|v| !v.is_empty()) {
            assert!(
                capability.ends_with("=()"),
                "{capability} is not denied outright"
            );
        }
        for expected in ["camera", "microphone", "geolocation", "payment", "usb"] {
            assert!(value.contains(expected), "{expected} is not named");
        }
    }
}
