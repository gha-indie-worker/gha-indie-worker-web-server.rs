#![forbid(unsafe_code)]
//! Configuration, read once at start-up from the environment Cloud Run and the k8s deployment
//! actually give this service.
//!
//! The variable names here are exactly the ones in
//! `gha-indie-worker-infra/gcp/cloudrun/services.tf` for `google_cloud_run_v2_service.web`, plus
//! the flags-2-env surface declared in `.cli-flags.toml`. Two consequences worth knowing:
//!
//! * `ORES_MIDDLEWARE_*` and `ORES_OTEL_*` are **not** read here. They belong to
//!   `ores-middleware` and `ores-otel`, which read them from the same environment themselves;
//!   re-reading them in the service is how the two copies drift apart.
//! * `GHA_INDIE_WORKER_APEX` is the one name this file reads that services.tf does not set. It is
//!   optional: when it is absent the apex is derived from `GHA_INDIE_WORKER_PUBLIC_URL`
//!   (`https://app.indiebuild.dev` → `indiebuild.dev`), so the deployed configuration keeps
//!   working unchanged and a developer can still point a laptop at another domain.
//!
//! [`WebConfig::from_map`] is the pure constructor: `main` hands it the environment after
//! flags-2-env has resolved argv over it, and tests hand it a literal map.
//! [`WebConfig::from_env`] is a convenience over the raw process environment.

use std::collections::BTreeMap;
use std::fmt;

/// A value that must never be printed. `Debug` is implemented by hand precisely so that a
/// `tracing::debug!(?config)` somewhere cannot leak a database URL into a log sink.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Secret(Option<String>);

impl Secret {
    /// Read a secret from a resolved environment map. The raw value is kept, blank or not, so
    /// that [`crate::server::startup_plan`] can refuse a blank one rather than silently dropping it.
    #[must_use]
    pub fn from_map(environment: &BTreeMap<String, String>, name: &str) -> Self {
        Self(environment.get(name).cloned())
    }

    #[must_use]
    pub fn from_env(name: &str) -> Self {
        Self(std::env::var(name).ok())
    }

    #[must_use]
    pub fn expose(&self) -> Option<&str> {
        self.0.as_deref()
    }

    /// Set to something other than whitespace.
    #[must_use]
    pub fn is_present(&self) -> bool {
        self.0.as_deref().is_some_and(|value| !value.trim().is_empty())
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
    /// `GHA_INDIE_WORKER_API_URL` — where the JSON/WebSocket API lives. Falls back to
    /// `GHA_INDIE_WORKER_API_HTTP_BASE`, then to `https://api.<apex>`.
    pub api_url: String,
    /// `GHA_INDIE_WORKER_API_HTTP_BASE` — the stateless-HTTP avenue, exactly as configured. Kept
    /// raw so a blank value fails [`crate::server::startup_plan`] instead of vanishing.
    pub api_http_base: Option<String>,
    /// `GHA_INDIE_WORKER_NATS_URL` — the durable NATS avenue. Credentials live in the URL's
    /// secret-store value, never in a CLI flag.
    pub nats_url: Option<String>,
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

    /// `DATABASE_URL` (the canonical Neon project, read-only for this tier), or the flags-2-env
    /// key `GHA_INDIE_WORKER_DATABASE_URL`.
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
    /// Build from a resolved environment map. Never panics: a missing optional variable degrades a
    /// feature, and a missing required one is reported by [`Self::warnings`] at start-up rather
    /// than at the first request.
    #[must_use]
    pub fn from_map(environment: &BTreeMap<String, String>) -> Self {
        let var = |name: &str| {
            environment
                .get(name)
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
        };
        let public_url = var("GHA_INDIE_WORKER_PUBLIC_URL")
            .unwrap_or_else(|| "http://127.0.0.1:8081".to_owned());
        let apex = var("GHA_INDIE_WORKER_APEX")
            .or_else(|| apex_from_public_url(&public_url))
            .unwrap_or_else(|| "indiebuild.dev".to_owned());
        let api_http_base = environment.get("GHA_INDIE_WORKER_API_HTTP_BASE").cloned();
        let database_url = if environment.contains_key("DATABASE_URL") {
            Secret::from_map(environment, "DATABASE_URL")
        } else {
            Secret::from_map(environment, "GHA_INDIE_WORKER_DATABASE_URL")
        };
        Self {
            bind: environment
                .get("GHA_INDIE_WORKER_WEB_BIND")
                .cloned()
                .unwrap_or_else(|| "127.0.0.1:8081".to_owned()),
            api_url: var("GHA_INDIE_WORKER_API_URL")
                .or_else(|| {
                    api_http_base
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| format!("https://api.{apex}")),
            api_http_base,
            nats_url: environment.get("GHA_INDIE_WORKER_NATS_URL").cloned(),
            apex,
            public_url,
            shared_auth_base_url: var("SHARED_AUTH_BASE_URL").unwrap_or_default(),
            shared_auth_issuer: var("SHARED_AUTH_ISSUER").unwrap_or_default(),
            shared_auth_audience: var("SHARED_AUTH_AUDIENCE")
                .unwrap_or_else(|| "indiebuild-web".to_owned()),
            ores_chat_base_url: var("ORES_CHAT_BASE_URL"),
            otel_service_name: var("ORES_OTEL_SERVICE_NAME")
                .unwrap_or_else(|| "gha-indie-worker-web-server".to_owned()),
            database_url,
            auth_database_url: Secret::from_map(environment, "AUTH_DATABASE_URL"),
            supabase_url: Secret::from_map(environment, "SUPABASE_URL"),
            supabase_anon_key: Secret::from_map(environment, "SUPABASE_ANON_KEY"),
            shared_auth_introspection_credential: Secret::from_map(
                environment,
                "SHARED_AUTH_INTROSPECTION_CREDENTIAL",
            ),
            edge_shared_secret: Secret::from_map(environment, "GHA_INDIE_WORKER_EDGE_SHARED_SECRET"),
            ores_chat_service_token: Secret::from_map(environment, "ORES_CHAT_SERVICE_TOKEN"),
        }
    }

    /// Read the raw process environment, without flags-2-env argv resolution.
    #[must_use]
    pub fn from_env() -> Self {
        Self::from_map(&std::env::vars().collect())
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
        assert!(!Secret(Some("   ".to_owned())).is_present());
    }

    #[test]
    fn a_plain_http_origin_disables_secure_cookies_and_the_domain_attribute() {
        let mut config = WebConfig::from_map(&BTreeMap::new());
        config.public_url = "http://127.0.0.1:8081".to_owned();
        assert!(!config.cookies_are_secure());
        assert_eq!(config.cookie_domain(), None);
        config.public_url = "https://app.indiebuild.dev".to_owned();
        config.apex = "indiebuild.dev".to_owned();
        assert!(config.cookies_are_secure());
        assert_eq!(config.cookie_domain(), Some("indiebuild.dev"));
    }

    #[test]
    fn flags_2_env_keys_and_deployment_keys_both_reach_the_config() {
        let environment = BTreeMap::from([
            ("GHA_INDIE_WORKER_WEB_BIND".to_owned(), "0.0.0.0:8080".to_owned()),
            ("GHA_INDIE_WORKER_API_HTTP_BASE".to_owned(), "http://api:8080".to_owned()),
            ("GHA_INDIE_WORKER_NATS_URL".to_owned(), "nats://127.0.0.1:4222".to_owned()),
            ("GHA_INDIE_WORKER_DATABASE_URL".to_owned(), "postgres://flag".to_owned()),
            ("GHA_INDIE_WORKER_PUBLIC_URL".to_owned(), "https://app.indiebuild.dev".to_owned()),
        ]);
        let config = WebConfig::from_map(&environment);
        assert_eq!(config.bind, "0.0.0.0:8080");
        assert_eq!(config.api_http_base.as_deref(), Some("http://api:8080"));
        assert_eq!(config.api_url, "http://api:8080");
        assert_eq!(config.nats_url.as_deref(), Some("nats://127.0.0.1:4222"));
        assert_eq!(config.database_url.expose(), Some("postgres://flag"));
        assert_eq!(config.apex, "indiebuild.dev");

        let mut with_canonical = environment;
        with_canonical.insert("DATABASE_URL".to_owned(), "postgres://canonical".to_owned());
        assert_eq!(
            WebConfig::from_map(&with_canonical).database_url.expose(),
            Some("postgres://canonical"),
            "the deployment's DATABASE_URL wins over the flags-2-env key"
        );
    }
}
