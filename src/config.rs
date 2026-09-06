#![forbid(unsafe_code)]
//! Configuration, read once at start-up from the environment Cloud Run and the k8s deployment
//! actually give this service.
//!
//! The variable names here are exactly the ones in
//! `gha-indie-worker-infra/gcp/cloudrun/services.tf` for `google_cloud_run_v2_service.web`, and
//! deliberately nothing else. Two consequences worth knowing:
//!
//! * `ORES_MIDDLEWARE_*` and `ORES_OTEL_*` are **not** read here. They belong to
//!   `ores-middleware` and `ores-otel`, which read them from the same environment themselves;
//!   re-reading them in the service is how the two copies drift apart.
//! * `GHA_INDIE_WORKER_APEX` is the one name this file reads that services.tf does not set. It is
//!   optional: when it is absent the apex is derived from `GHA_INDIE_WORKER_PUBLIC_URL`
//!   (`https://app.indiebuild.dev` → `indiebuild.dev`), so the deployed configuration keeps
//!   working unchanged and a developer can still point a laptop at another domain.

use std::fmt;

/// A value that must never be printed. `Debug` is implemented by hand precisely so that a
/// `tracing::debug!(?config)` somewhere cannot leak a database URL into a log sink.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Secret(Option<String>);

impl Secret {
    #[must_use]
    pub fn from_env(name: &str) -> Self {
        Self(
            std::env::var(name)
                .ok()
                .filter(|value| !value.trim().is_empty()),
        )
    }

    #[must_use]
    pub fn expose(&self) -> Option<&str> {
        self.0.as_deref()
    }

    #[must_use]
    pub const fn is_present(&self) -> bool {
        self.0.is_some()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(if self.0.is_some() {
            "Secret(set)"
        } else {
            "Secret(unset)"
        })
    }
}

/// Everything the web server needs to boot.
#[derive(Clone, Debug)]
pub struct WebConfig {
    /// `GHA_INDIE_WORKER_WEB_BIND`
    pub bind: String,
    /// `GHA_INDIE_WORKER_PUBLIC_URL` — the canonical origin of the app shell.
    pub public_url: String,
    /// `GHA_INDIE_WORKER_API_URL` — where the JSON/WebSocket API lives.
    pub api_url: String,
    /// `GHA_INDIE_WORKER_APEX`, or derived from `public_url`.
    pub apex: String,
    /// `SHARED_AUTH_BASE_URL`
    pub shared_auth_base_url: String,
    /// `SHARED_AUTH_ISSUER` — checked exactly by `authz::project`.
    pub shared_auth_issuer: String,
    /// `SHARED_AUTH_AUDIENCE` — must be `indiebuild-web` for this binary.
    pub shared_auth_audience: String,
    /// `ORES_CHAT_BASE_URL`
    pub ores_chat_base_url: Option<String>,
    /// `ORES_OTEL_SERVICE_NAME`
    pub otel_service_name: String,

    /// `DATABASE_URL` — the canonical Neon project, read-only for this tier.
    pub database_url: Secret,
    /// `AUTH_DATABASE_URL`
    pub auth_database_url: Secret,
    /// `SUPABASE_URL`
    pub supabase_url: Secret,
    /// `SUPABASE_ANON_KEY`
    pub supabase_anon_key: Secret,
    /// `SHARED_AUTH_INTROSPECTION_CREDENTIAL`
    pub shared_auth_introspection_credential: Secret,
    /// `GHA_INDIE_WORKER_EDGE_SHARED_SECRET` — proves a request came through our edge Worker.
    pub edge_shared_secret: Secret,
    /// `ORES_CHAT_SERVICE_TOKEN`
    pub ores_chat_service_token: Secret,
}

impl WebConfig {
    /// Read the environment. Never panics: a missing optional variable degrades a feature, and a
    /// missing required one is reported by [`Self::warnings`] at start-up rather than at the first
    /// request.
    #[must_use]
    pub fn from_env() -> Self {
        let public_url = var("GHA_INDIE_WORKER_PUBLIC_URL")
            .unwrap_or_else(|| "http://127.0.0.1:8081".to_owned());
        let apex = var("GHA_INDIE_WORKER_APEX")
            .or_else(|| apex_from_public_url(&public_url))
            .unwrap_or_else(|| "indiebuild.dev".to_owned());
        Self {
            bind: var("GHA_INDIE_WORKER_WEB_BIND").unwrap_or_else(|| "127.0.0.1:8081".to_owned()),
            api_url: var("GHA_INDIE_WORKER_API_URL")
                .unwrap_or_else(|| format!("https://api.{apex}")),
            apex,
            public_url,
            shared_auth_base_url: var("SHARED_AUTH_BASE_URL").unwrap_or_default(),
            shared_auth_issuer: var("SHARED_AUTH_ISSUER").unwrap_or_default(),
            shared_auth_audience: var("SHARED_AUTH_AUDIENCE")
                .unwrap_or_else(|| "indiebuild-web".to_owned()),
            ores_chat_base_url: var("ORES_CHAT_BASE_URL"),
            otel_service_name: var("ORES_OTEL_SERVICE_NAME")
                .unwrap_or_else(|| "gha-indie-worker-web-server".to_owned()),
            database_url: Secret::from_env("DATABASE_URL"),
            auth_database_url: Secret::from_env("AUTH_DATABASE_URL"),
            supabase_url: Secret::from_env("SUPABASE_URL"),
            supabase_anon_key: Secret::from_env("SUPABASE_ANON_KEY"),
            shared_auth_introspection_credential: Secret::from_env(
                "SHARED_AUTH_INTROSPECTION_CREDENTIAL",
            ),
            edge_shared_secret: Secret::from_env("GHA_INDIE_WORKER_EDGE_SHARED_SECRET"),
            ores_chat_service_token: Secret::from_env("ORES_CHAT_SERVICE_TOKEN"),
        }
    }

    /// Cookies are `Secure` unless the public origin is plain http, which only a laptop is.
    #[must_use]
    pub fn cookies_are_secure(&self) -> bool {
        self.public_url.starts_with("https://")
    }

    /// The `Domain` attribute for session and CSRF cookies: the apex, so that signing in at
    /// `user.` is visible at `app.`.
    #[must_use]
    pub fn cookie_domain(&self) -> Option<&str> {
        if self.cookies_are_secure() {
            Some(self.apex.as_str())
        } else {
            None
        }
    }

    /// Whether this process is talking to a real shared-auth deployment. When false the magic-link
    /// flow runs against the in-process development stub and says so on every page.
    #[must_use]
    pub fn shared_auth_configured(&self) -> bool {
        !self.shared_auth_base_url.is_empty()
            && !self.shared_auth_issuer.is_empty()
            && self.shared_auth_introspection_credential.is_present()
    }

    /// Configuration problems worth shouting about at boot, in the order they will bite.
    #[must_use]
    pub fn warnings(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !self.shared_auth_configured() {
            out.push(
                "shared-auth is not configured (SHARED_AUTH_BASE_URL / SHARED_AUTH_ISSUER / \
                 SHARED_AUTH_INTROSPECTION_CREDENTIAL); the development sign-in stub is active"
                    .to_owned(),
            );
        }
        if self.shared_auth_audience != "indiebuild-web" {
            out.push(format!(
                "SHARED_AUTH_AUDIENCE is {:?}; this binary is the web surface and should present \
                 indiebuild-web",
                self.shared_auth_audience
            ));
        }
        if !self.database_url.is_present() {
            out.push(
                "DATABASE_URL is unset; read-only projections fall back to fixtures".to_owned(),
            );
        }
        if !self.edge_shared_secret.is_present() {
            out.push(
                "GHA_INDIE_WORKER_EDGE_SHARED_SECRET is unset; requests cannot be proven to have \
                 come through the edge Worker"
                    .to_owned(),
            );
        }
        if !self.cookies_are_secure() {
            out.push(format!(
                "GHA_INDIE_WORKER_PUBLIC_URL is {:?}; cookies will be issued without Secure",
                self.public_url
            ));
        }
        out
    }
}

fn var(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// `https://app.indiebuild.dev` → `indiebuild.dev`. A bare apex (`https://indiebuild.dev`) is
/// returned unchanged, and anything with fewer than two labels is not an apex at all.
fn apex_from_public_url(public_url: &str) -> Option<String> {
    let rest = public_url.split("://").nth(1).unwrap_or(public_url);
    let host = rest
        .split('/')
        .next()?
        .split(':')
        .next()?
        .trim_end_matches('.');
    if host.is_empty() || host.parse::<std::net::IpAddr>().is_ok() {
        return None;
    }
    let labels: Vec<&str> = host.split('.').collect();
    match labels.as_slice() {
        [] | [_] => None,
        [_, _] => Some(host.to_ascii_lowercase()),
        // Drop exactly one leading label: `app.indiebuild.dev` is a surface of `indiebuild.dev`.
        [_, tail @ ..] => Some(tail.join(".").to_ascii_lowercase()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_apex_is_derived_from_the_public_url_when_it_is_not_set() {
        assert_eq!(
            apex_from_public_url("https://app.indiebuild.dev").as_deref(),
            Some("indiebuild.dev")
        );
        assert_eq!(
            apex_from_public_url("https://APP.IndieBuild.dev/").as_deref(),
            Some("indiebuild.dev")
        );
        assert_eq!(
            apex_from_public_url("https://indiebuild.dev").as_deref(),
            Some("indiebuild.dev")
        );
        assert_eq!(apex_from_public_url("http://127.0.0.1:8081"), None);
        assert_eq!(apex_from_public_url("http://localhost:8081"), None);
    }

    #[test]
    fn secrets_never_print_themselves() {
        let secret = Secret(Some("postgres://user:password@host/db".to_owned()));
        assert_eq!(format!("{secret:?}"), "Secret(set)");
        assert_eq!(format!("{:?}", Secret(None)), "Secret(unset)");
        assert_eq!(secret.expose(), Some("postgres://user:password@host/db"));
    }

    #[test]
    fn a_plain_http_origin_disables_secure_cookies_and_the_domain_attribute() {
        let mut config = WebConfig::from_env();
        config.public_url = "http://127.0.0.1:8081".to_owned();
        assert!(!config.cookies_are_secure());
        assert_eq!(config.cookie_domain(), None);
        config.public_url = "https://app.indiebuild.dev".to_owned();
        config.apex = "indiebuild.dev".to_owned();
        assert!(config.cookies_are_secure());
        assert_eq!(config.cookie_domain(), Some("indiebuild.dev"));
    }
}
