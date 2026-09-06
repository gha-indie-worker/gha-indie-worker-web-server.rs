#![forbid(unsafe_code)]

//! Host routing.
//!
//! One process, four public hosts. The `Host` header decides which [`Surface`]
//! serves the request; `X-Forwarded-Host` is honoured **only** when the peer
//! address is inside a trusted-proxy CIDR (Cloudflare), because otherwise any
//! client could pick its own surface — and the org surface is where seats,
//! billing and member roles live.
//!
//! An unrecognised host is [`Surface::Unknown`] and gets a 404 page. It is never
//! silently treated as `app.`.

pub mod app;
pub mod common;
pub mod mobile;
pub mod org;
pub mod unknown;
pub mod user;

use std::net::IpAddr;
use std::sync::Arc;

use axum::extract::Request;
use axum::http::HeaderMap;
use axum::Router;
use tower::ServiceExt;

use crate::state::AppState;

/// Cloudflare's published proxy ranges, used when
/// `GHA_INDIE_WORKER_TRUSTED_PROXY_CIDRS` is unset. Cloudflare is the only
/// proxy in front of indiebuild.dev.
pub const CLOUDFLARE_CIDRS: &[&str] = &[
    "173.245.48.0/20",
    "103.21.244.0/22",
    "103.22.200.0/22",
    "103.31.4.0/22",
    "141.101.64.0/18",
    "108.162.192.0/18",
    "190.93.240.0/20",
    "188.114.96.0/20",
    "197.234.240.0/22",
    "198.41.128.0/17",
    "162.158.0.0/15",
    "104.16.0.0/13",
    "104.24.0.0/14",
    "172.64.0.0/13",
    "131.0.72.0/22",
    "2400:cb00::/32",
    "2606:4700::/32",
    "2803:f800::/32",
    "2405:b500::/32",
    "2405:8100::/32",
    "2a06:98c0::/29",
    "2c0f:f248::/32",
];

/// Which product surface a request is for.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Surface {
    /// `app.` — marketing-continuous home plus the product dashboard.
    App,
    /// `user.` — B2C signup, login, personal workspace, API tokens.
    User,
    /// `org.` — B2B login, onboarding wizard, members/roles/seats/audit/SSO.
    Org,
    /// `m.` — compact layouts of the same pages with a bottom navigation bar.
    Mobile,
    /// Anything else. Answered with a 404 page, never with product content.
    Unknown,
}

impl Surface {
    /// The host label (`app`, `user`, `org`, `m`).
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::App => "app",
            Self::User => "user",
            Self::Org => "org",
            Self::Mobile => "m",
            Self::Unknown => "unknown",
        }
    }

    #[must_use]
    pub fn from_label(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "app" => Some(Self::App),
            "user" => Some(Self::User),
            "org" => Some(Self::Org),
            "m" | "mobile" => Some(Self::Mobile),
            _ => None,
        }
    }

    /// `app.indiebuild.dev`
    #[must_use]
    pub fn host(self, base_domain: &str) -> String {
        format!("{}.{}", self.label(), base_domain)
    }

    /// `https://app.indiebuild.dev`
    #[must_use]
    pub fn origin(self, base_domain: &str) -> String {
        format!("https://{}", self.host(base_domain))
    }

    /// Mobile gets bottom navigation and no hover-only affordances.
    #[must_use]
    pub const fn is_compact(self) -> bool {
        matches!(self, Self::Mobile)
    }

    /// Surfaces that can hold a signed-in product session.
    #[must_use]
    pub const fn is_authenticated_surface(self) -> bool {
        matches!(self, Self::App | Self::User | Self::Org | Self::Mobile)
    }

    /// Where a signed-out visitor on this surface is sent to log in.
    #[must_use]
    pub fn login_origin(self, base_domain: &str) -> String {
        match self {
            // app. and m. do not own credentials; personal login lives on user.
            Self::App | Self::Mobile | Self::User | Self::Unknown => Self::User.origin(base_domain),
            Self::Org => Self::Org.origin(base_domain),
        }
    }
}

/// One CIDR block, matched bitwise so IPv4 and IPv6 use the same code path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Cidr {
    addr: IpAddr,
    bits: u8,
}

impl Cidr {
    /// Parses `10.0.0.0/8` or `2400:cb00::/32`. A bare address is an exact match.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        let (addr_part, bits_part) = match raw.split_once('/') {
            Some((a, b)) => (a, Some(b)),
            None => (raw, None),
        };
        let addr: IpAddr = addr_part.parse().ok()?;
        let max = if addr.is_ipv4() { 32 } else { 128 };
        let bits = match bits_part {
            Some(b) => b.trim().parse::<u8>().ok()?,
            None => max,
        };
        if bits > max {
            return None;
        }
        Some(Self { addr, bits })
    }

    #[must_use]
    pub fn contains(&self, candidate: IpAddr) -> bool {
        let candidate = normalize(candidate);
        let network = normalize(self.addr);
        match (network, candidate) {
            (IpAddr::V4(net), IpAddr::V4(ip)) => prefix_eq(&net.octets(), &ip.octets(), self.bits),
            (IpAddr::V6(net), IpAddr::V6(ip)) => prefix_eq(&net.octets(), &ip.octets(), self.bits),
            _ => false,
        }
    }
}

/// IPv4-mapped IPv6 (`::ffff:1.2.3.4`) compares as the IPv4 address it carries.
fn normalize(addr: IpAddr) -> IpAddr {
    match addr {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(IpAddr::V6(v6), IpAddr::V4),
        other => other,
    }
}

fn prefix_eq(network: &[u8], candidate: &[u8], bits: u8) -> bool {
    let full = usize::from(bits / 8);
    let remainder = bits % 8;
    if network.len() != candidate.len() || full > network.len() {
        return false;
    }
    if network[..full] != candidate[..full] {
        return false;
    }
    if remainder == 0 {
        return true;
    }
    let mask = 0xffu8 << (8 - remainder);
    (network[full] & mask) == (candidate[full] & mask)
}

/// How this deployment turns a request into a [`Surface`].
#[derive(Clone, Debug)]
pub struct HostPolicy {
    pub base_domain: String,
    pub trusted_proxies: Vec<Cidr>,
    /// Surface used for loopback / `localhost` so one local process is usable.
    pub dev_surface: Surface,
}

impl HostPolicy {
    #[must_use]
    pub fn new(base_domain: impl Into<String>, cidrs: &[String], dev_surface: Surface) -> Self {
        Self {
            base_domain: base_domain.into(),
            trusted_proxies: cidrs.iter().filter_map(|c| Cidr::parse(c)).collect(),
            dev_surface,
        }
    }

    #[must_use]
    pub fn trusts(&self, peer: Option<IpAddr>) -> bool {
        peer.is_some_and(|ip| self.trusted_proxies.iter().any(|cidr| cidr.contains(ip)))
    }

    /// The host this request is really for.
    ///
    /// `X-Forwarded-Host` wins only behind a trusted proxy; otherwise the `Host`
    /// header (or HTTP/2 `:authority`, which axum normalises into `host`) is used.
    #[must_use]
    pub fn effective_host(&self, headers: &HeaderMap, peer: Option<IpAddr>) -> Option<String> {
        let forwarded = if self.trusts(peer) {
            headers
                .get("x-forwarded-host")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.split(',').next())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
        } else {
            None
        };
        forwarded
            .or_else(|| {
                headers
                    .get(axum::http::header::HOST)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_owned)
            })
            .map(|host| normalize_host(&host))
    }

    /// Maps a host to a surface. Unknown hosts are [`Surface::Unknown`].
    #[must_use]
    pub fn surface_for_host(&self, host: &str) -> Surface {
        let host = normalize_host(host);
        if host.is_empty() {
            return Surface::Unknown;
        }
        if is_loopback_host(&host) {
            return self.dev_surface;
        }
        if host == self.base_domain || host == format!("www.{}", self.base_domain) {
            // The apex and www are marketing; they behave as app.
            return Surface::App;
        }
        match host.strip_suffix(&format!(".{}", self.base_domain)) {
            Some(label) if !label.contains('.') => Surface::from_label(label).unwrap_or(Surface::Unknown),
            _ => Surface::Unknown,
        }
    }

    #[must_use]
    pub fn resolve(&self, headers: &HeaderMap, peer: Option<IpAddr>) -> Surface {
        self.effective_host(headers, peer)
            .map_or(Surface::Unknown, |host| self.surface_for_host(&host))
    }
}

/// Lowercases and strips the port. `App.IndieBuild.dev:443` → `app.indiebuild.dev`.
#[must_use]
pub fn normalize_host(raw: &str) -> String {
    let host = raw.trim().trim_end_matches('.').to_ascii_lowercase();
    // Bracketed IPv6 literal: keep the brackets, drop only a trailing :port.
    if let Some(rest) = host.strip_prefix('[') {
        return match rest.split_once(']') {
            Some((inner, _)) => format!("[{inner}]"),
            None => host,
        };
    }
    match host.rsplit_once(':') {
        Some((name, port)) if port.chars().all(|c| c.is_ascii_digit()) && !port.is_empty() => name.to_owned(),
        _ => host,
    }
}

fn is_loopback_host(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "0.0.0.0" | "[::1]" | "::1") || host.ends_with(".localhost")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> HostPolicy {
        HostPolicy::new(
            "indiebuild.dev",
            &["173.245.48.0/20".to_owned(), "2400:cb00::/32".to_owned()],
            Surface::App,
        )
    }

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(
                axum::http::HeaderName::from_bytes(name.as_bytes()).expect("static header name"),
                axum::http::HeaderValue::from_str(value).expect("static header value"),
            );
        }
        map
    }

    #[test]
    fn each_product_host_resolves_to_its_surface() {
        let policy = policy();
        assert_eq!(policy.surface_for_host("app.indiebuild.dev"), Surface::App);
        assert_eq!(policy.surface_for_host("user.indiebuild.dev"), Surface::User);
        assert_eq!(policy.surface_for_host("org.indiebuild.dev"), Surface::Org);
        assert_eq!(policy.surface_for_host("m.indiebuild.dev"), Surface::Mobile);
        assert_eq!(policy.surface_for_host("indiebuild.dev"), Surface::App);
        assert_eq!(policy.surface_for_host("www.indiebuild.dev"), Surface::App);
    }

    #[test]
    fn unknown_and_lookalike_hosts_are_never_a_product_surface() {
        let policy = policy();
        assert_eq!(policy.surface_for_host("evil.com"), Surface::Unknown);
        assert_eq!(policy.surface_for_host("admin.indiebuild.dev"), Surface::Unknown);
        assert_eq!(policy.surface_for_host("app.indiebuild.dev.evil.com"), Surface::Unknown);
        assert_eq!(policy.surface_for_host("a.b.indiebuild.dev"), Surface::Unknown);
        assert_eq!(policy.surface_for_host(""), Surface::Unknown);
    }

    #[test]
    fn host_is_case_and_port_insensitive() {
        assert_eq!(normalize_host("App.IndieBuild.dev:443"), "app.indiebuild.dev");
        assert_eq!(normalize_host("[::1]:8081"), "[::1]");
        assert_eq!(policy().surface_for_host("ORG.indiebuild.dev:8443"), Surface::Org);
    }

    #[test]
    fn forwarded_host_is_honoured_only_behind_a_trusted_proxy() {
        let policy = policy();
        let headers = headers(&[
            ("host", "app.indiebuild.dev"),
            ("x-forwarded-host", "org.indiebuild.dev"),
        ]);

        let untrusted: IpAddr = "203.0.113.9".parse().expect("literal");
        assert_eq!(policy.resolve(&headers, Some(untrusted)), Surface::App);
        assert_eq!(policy.resolve(&headers, None), Surface::App);

        let cloudflare: IpAddr = "173.245.48.7".parse().expect("literal");
        assert_eq!(policy.resolve(&headers, Some(cloudflare)), Surface::Org);
    }

    #[test]
    fn ipv6_and_mapped_proxies_match_their_cidr() {
        let policy = policy();
        let v6: IpAddr = "2400:cb00::1".parse().expect("literal");
        assert!(policy.trusts(Some(v6)));
        let mapped: IpAddr = "::ffff:173.245.48.7".parse().expect("literal");
        assert!(policy.trusts(Some(mapped)));
        let outside: IpAddr = "2400:cb01::1".parse().expect("literal");
        assert!(!policy.trusts(Some(outside)));
    }

    #[test]
    fn loopback_uses_the_configured_development_surface() {
        let mut policy = policy();
        assert_eq!(policy.surface_for_host("localhost:8081"), Surface::App);
        policy.dev_surface = Surface::Org;
        assert_eq!(policy.surface_for_host("127.0.0.1:8081"), Surface::Org);
    }

    #[test]
    fn cidr_rejects_malformed_input() {
        assert!(Cidr::parse("not-an-ip/8").is_none());
        assert!(Cidr::parse("10.0.0.0/33").is_none());
        assert!(Cidr::parse("10.1.2.3").is_some());
    }
}

/// One built router per surface, plus the 404 surface for anything else.
///
/// The routers are built once at boot and cloned per request. Dispatching by
/// cloning a `Router` and calling it as a `Service` is what lets `org.` and
/// `user.` both own a `/login` that means different things, without prefixing
/// either of them.
pub struct SurfaceRouters {
    app: Router,
    user: Router,
    org: Router,
    mobile: Router,
    unknown: Router,
}

impl SurfaceRouters {
    #[must_use]
    pub fn build(state: &AppState) -> Self {
        Self {
            app: app::router(state.clone()),
            user: user::router(state.clone()),
            org: org::router(state.clone()),
            mobile: mobile::router(state.clone()),
            unknown: unknown::router(state.clone()),
        }
    }

    #[must_use]
    pub fn for_surface(&self, surface: Surface) -> &Router {
        match surface {
            Surface::App => &self.app,
            Surface::User => &self.user,
            Surface::Org => &self.org,
            Surface::Mobile => &self.mobile,
            Surface::Unknown => &self.unknown,
        }
    }
}

/// The host-routing entry point.
///
/// The surface was already resolved by [`crate::middleware::request_context`]
/// and put in the request's extensions, so dispatch never re-reads the `Host`
/// header — one resolution per request, one place where the trusted-proxy rule
/// is applied.
#[must_use]
pub fn dispatch_router(state: &AppState) -> Router {
    let routers = Arc::new(SurfaceRouters::build(state));
    Router::new().fallback(move |request: Request| {
        let routers = Arc::clone(&routers);
        async move {
            let surface = request
                .extensions()
                .get::<crate::middleware::RequestCtx>()
                .map_or(Surface::Unknown, |context| context.surface);
            match routers.for_surface(surface).clone().oneshot(request).await {
                Ok(response) => response,
                // A `Router`'s error type is `Infallible`: this arm is uninhabited.
                Err(error) => match error {},
            }
        }
    })
}
