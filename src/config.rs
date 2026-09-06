#![forbid(unsafe_code)]

//! Environment-only configuration. Every key is declared in `.cli-flags.toml`
//! and mirrored into `generated/` by flags-2-env. Secrets are read once here and
//! are never logged, never rendered into markup and never put in a URL.

use std::path::PathBuf;

use thiserror::Error;

use crate::hosts::Surface;

/// Default apex the four product hosts hang off.
pub const DEFAULT_BASE_DOMAIN: &str = "indiebuild.dev";
/// Default listen address (Cloud Run overrides with `PORT`).
pub const DEFAULT_BIND: &str = "127.0.0.1:8081";
/// Default cookie name. Host-scoped: no `Domain` attribute is ever emitted.
pub const DEFAULT_SESSION_COOKIE: &str = "giw_session";
/// Default CSRF cookie name (readable by htmx, hence not `HttpOnly`).
pub const DEFAULT_CSRF_COOKIE: &str = "giw_csrf";

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("{0} is required in production")]
    MissingInProduction(&'static str),
    #[error("{key} is invalid: {reason}")]
    Invalid { key: &'static str, reason: &'static str },
}

/// Which runtime posture the process is in. Mirrors `ORES_MIDDLEWARE_ENV`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Environment {
    Development,
    Test,
    Staging,
    Production,
}

impl Environment {
    #[must_use]
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "production" | "prod" => Self::Production,
            "staging" | "stage" => Self::Staging,
            "test" | "testing" => Self::Test,
            _ => Self::Development,
        }
    }

    #[must_use]
    pub const fn is_production(self) -> bool {
        matches!(self, Self::Production)
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Development => "development",
            Self::Test => "test",
            Self::Staging => "staging",
            Self::Production => "production",
        }
    }
}

/// shared-auth wiring for the **product** instance. Admin bases are deliberately
/// absent: this server must never accept an admin-instance token.
#[derive(Clone, Debug, Default)]
pub struct SharedAuthConfig {
    pub base: Option<String>,
    pub audience: Option<String>,
    pub introspect_secret: Option<String>,
}

impl SharedAuthConfig {
    #[must_use]
    pub fn is_configured(&self) -> bool {
        self.base.is_some()
    }
}

/// Federated-JWT verification (shared-auth federates Supabase Auth and Neon Auth).
#[derive(Clone, Debug, Default)]
pub struct JwtConfig {
    pub supabase_url: Option<String>,
    pub supabase_jwks_url: Option<String>,
    pub neon_auth_url: Option<String>,
    pub neon_auth_jwks_url: Option<String>,
    pub accepted_issuers: Vec<String>,
    pub accepted_audiences: Vec<String>,
    pub jwks_ttl_seconds: u64,
}

#[derive(Clone, Debug)]
pub struct WebConfig {
    pub environment: Environment,
    pub bind: String,
    pub base_domain: String,
    /// Surface used for `localhost`/loopback so a single local process is usable.
    pub dev_surface: Surface,
    /// CIDRs whose `X-Forwarded-Host` / `X-Forwarded-For` we honour. Cloudflare.
    pub trusted_proxy_cidrs: Vec<String>,

    /// Kept from the skeleton: legacy alias for `api_http_base`.
    pub api_base: Option<String>,
    /// api-server origin for every write and for the WebSocket relay.
    pub api_http_base: String,
    /// Kept from the skeleton.
    pub database_url: Option<String>,
    /// Read-only canonical pool. Reads only; this process never runs DDL.
    pub database_url_canonical: Option<String>,

    pub assets_dir: PathBuf,
    pub release_manifest_url: Option<String>,

    pub session_secret: Option<String>,
    pub session_cookie_name: String,
    pub csrf_cookie_name: String,
    pub session_ttl_seconds: u64,

    pub shared_auth: SharedAuthConfig,
    pub jwt: JwtConfig,

    pub chat_enabled: bool,
    pub rate_limit_hmac_secret: Option<String>,

    pub tcp_bind: Option<String>,
    pub nats_url: Option<String>,

    /// Kept from the skeleton.
    pub json: bool,
}

fn var(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
}

fn flag(key: &str, default: bool) -> bool {
    match var(key).map(|v| v.to_ascii_lowercase()) {
        Some(v) => matches!(v.as_str(), "1" | "true" | "yes" | "on"),
        None => default,
    }
}

fn number(key: &str, default: u64) -> u64 {
    var(key).and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn csv(key: &str) -> Vec<String> {
    var(key)
        .map(|v| {
            v.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

impl WebConfig {
    /// Reads the process environment. Never fails, so a misconfigured value is
    /// reported by [`Self::validate`] with the key name rather than by a panic.
    #[must_use]
    pub fn from_env() -> Self {
        let environment = Environment::parse(
            &var("GHA_INDIE_WORKER_ENV")
                .or_else(|| var("ORES_MIDDLEWARE_ENV"))
                .or_else(|| var("APP_ENV"))
                .unwrap_or_else(|| "development".into()),
        );
        // Cloud Run injects PORT; honour it before the explicit bind.
        let bind = var("GHA_INDIE_WORKER_WEB_BIND")
            .or_else(|| var("PORT").map(|p| format!("0.0.0.0:{p}")))
            .unwrap_or_else(|| DEFAULT_BIND.to_owned());
        let api_base = var("GHA_INDIE_WORKER_API_BASE");
        let api_http_base = var("GHA_INDIE_WORKER_API_HTTP_BASE")
            .or_else(|| api_base.clone())
            .unwrap_or_else(|| "http://127.0.0.1:8080".to_owned());

        Self {
            environment,
            bind,
            base_domain: var("GHA_INDIE_WORKER_BASE_DOMAIN").unwrap_or_else(|| DEFAULT_BASE_DOMAIN.to_owned()),
            dev_surface: var("GHA_INDIE_WORKER_DEV_SURFACE")
                .and_then(|v| Surface::from_label(&v))
                .unwrap_or(Surface::App),
            trusted_proxy_cidrs: {
                let configured = csv("GHA_INDIE_WORKER_TRUSTED_PROXY_CIDRS");
                if configured.is_empty() {
                    crate::hosts::CLOUDFLARE_CIDRS.iter().map(|s| (*s).to_owned()).collect()
                } else {
                    configured
                }
            },
            api_base,
            api_http_base,
            database_url: var("GHA_INDIE_WORKER_DATABASE_URL"),
            database_url_canonical: var("DATABASE_URL_CANONICAL")
                .or_else(|| var("GHA_INDIE_WORKER_DATABASE_URL_CANONICAL")),
            assets_dir: var("GHA_INDIE_WORKER_ASSETS_DIR").map_or_else(|| PathBuf::from("assets"), PathBuf::from),
            release_manifest_url: var("GHA_INDIE_WORKER_RELEASE_MANIFEST_URL"),
            session_secret: var("GHA_INDIE_WORKER_SESSION_SECRET"),
            session_cookie_name: var("GHA_INDIE_WORKER_SESSION_COOKIE_NAME")
                .unwrap_or_else(|| DEFAULT_SESSION_COOKIE.to_owned()),
            csrf_cookie_name: var("GHA_INDIE_WORKER_CSRF_COOKIE_NAME")
                .unwrap_or_else(|| DEFAULT_CSRF_COOKIE.to_owned()),
            session_ttl_seconds: number("GHA_INDIE_WORKER_SESSION_TTL_SECONDS", 60 * 60 * 12),
            shared_auth: SharedAuthConfig {
                base: var("SHARED_AUTH_BASE"),
                audience: var("SHARED_AUTH_AUDIENCE"),
                introspect_secret: var("SHARED_AUTH_INTROSPECT_SECRET"),
            },
            jwt: JwtConfig {
                supabase_url: var("SUPABASE_URL"),
                supabase_jwks_url: var("SUPABASE_JWKS_URL").or_else(|| {
                    var("SUPABASE_URL").map(|u| format!("{}/auth/v1/.well-known/jwks.json", u.trim_end_matches('/')))
                }),
                neon_auth_url: var("NEON_AUTH_URL"),
                neon_auth_jwks_url: var("NEON_AUTH_JWKS_URL"),
                accepted_issuers: csv("GHA_INDIE_WORKER_JWT_ISSUERS"),
                accepted_audiences: {
                    let configured = csv("GHA_INDIE_WORKER_JWT_AUDIENCES");
                    if configured.is_empty() {
                        var("SHARED_AUTH_AUDIENCE").into_iter().collect()
                    } else {
                        configured
                    }
                },
                jwks_ttl_seconds: number("GHA_INDIE_WORKER_JWKS_TTL_SECONDS", 300),
            },
            chat_enabled: flag("GHA_INDIE_WORKER_CHAT_ENABLED", true),
            rate_limit_hmac_secret: var("GHA_INDIE_WORKER_RATE_LIMIT_HMAC_SECRET")
                .or_else(|| var("ORES_MIDDLEWARE_RATE_LIMIT_HMAC_SECRET")),
            tcp_bind: var("GHA_INDIE_WORKER_WEB_TCP_BIND"),
            nats_url: var("GHA_INDIE_WORKER_NATS_URL"),
            json: flag("GHA_INDIE_WORKER_JSON", false),
        }
    }

    /// A self-contained configuration for unit and HTTP tests. No network, no
    /// database, no secrets from the environment.
    #[must_use]
    pub fn for_tests() -> Self {
        Self {
            environment: Environment::Test,
            bind: DEFAULT_BIND.to_owned(),
            base_domain: DEFAULT_BASE_DOMAIN.to_owned(),
            dev_surface: Surface::App,
            trusted_proxy_cidrs: vec!["127.0.0.0/8".into(), "::1/128".into()],
            api_base: None,
            api_http_base: "http://127.0.0.1:8080".to_owned(),
            database_url: None,
            database_url_canonical: None,
            assets_dir: PathBuf::from("assets"),
            release_manifest_url: None,
            session_secret: Some("test-session-secret-not-for-production".to_owned()),
            session_cookie_name: DEFAULT_SESSION_COOKIE.to_owned(),
            csrf_cookie_name: DEFAULT_CSRF_COOKIE.to_owned(),
            session_ttl_seconds: 3_600,
            shared_auth: SharedAuthConfig::default(),
            jwt: JwtConfig {
                jwks_ttl_seconds: 300,
                ..JwtConfig::default()
            },
            chat_enabled: true,
            rate_limit_hmac_secret: Some("test-rate-limit-secret".to_owned()),
            tcp_bind: None,
            nats_url: None,
            json: false,
        }
    }

    /// Fail closed on anything that would silently weaken production.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if !self.bind.contains(':') {
            return Err(ConfigError::Invalid {
                key: "GHA_INDIE_WORKER_WEB_BIND",
                reason: "expected host:port",
            });
        }
        if self.base_domain.is_empty() || self.base_domain.contains('/') {
            return Err(ConfigError::Invalid {
                key: "GHA_INDIE_WORKER_BASE_DOMAIN",
                reason: "expected a bare apex domain",
            });
        }
        if self.environment.is_production() {
            if self.session_secret.is_none() {
                return Err(ConfigError::MissingInProduction("GHA_INDIE_WORKER_SESSION_SECRET"));
            }
            if !self.api_http_base.starts_with("https://") {
                return Err(ConfigError::Invalid {
                    key: "GHA_INDIE_WORKER_API_HTTP_BASE",
                    reason: "production requires https://",
                });
            }
            if !self.shared_auth.is_configured() {
                return Err(ConfigError::MissingInProduction("SHARED_AUTH_BASE"));
            }
            if self.rate_limit_hmac_secret.is_none() {
                return Err(ConfigError::MissingInProduction(
                    "GHA_INDIE_WORKER_RATE_LIMIT_HMAC_SECRET",
                ));
            }
        }
        Ok(())
    }

    /// `https://api.indiebuild.dev/v1/ws` → the WebSocket origin of the api-server.
    #[must_use]
    pub fn api_ws_base(&self) -> String {
        let base = self.api_http_base.trim_end_matches('/');
        if let Some(rest) = base.strip_prefix("https://") {
            format!("wss://{rest}")
        } else if let Some(rest) = base.strip_prefix("http://") {
            format!("ws://{rest}")
        } else {
            format!("wss://{base}")
        }
    }

    /// Cookies are only marked `Secure` where TLS actually exists.
    #[must_use]
    pub const fn cookies_secure(&self) -> bool {
        !matches!(self.environment, Environment::Development | Environment::Test)
    }

    #[must_use]
    pub fn origin(&self, surface: Surface) -> String {
        surface.origin(&self.base_domain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn websocket_base_follows_the_api_scheme() {
        let mut config = WebConfig::for_tests();
        config.api_http_base = "https://api.indiebuild.dev/".into();
        assert_eq!(config.api_ws_base(), "wss://api.indiebuild.dev");
        config.api_http_base = "http://127.0.0.1:8080".into();
        assert_eq!(config.api_ws_base(), "ws://127.0.0.1:8080");
    }

    #[test]
    fn production_requires_the_session_secret() {
        let mut config = WebConfig::for_tests();
        config.environment = Environment::Production;
        config.session_secret = None;
        assert!(
            matches!(config.validate(), Err(ConfigError::MissingInProduction(key)) if key.ends_with("SESSION_SECRET"))
        );
    }

    #[test]
    fn test_configuration_validates() {
        assert!(WebConfig::for_tests().validate().is_ok());
    }

    #[test]
    fn cookies_are_insecure_only_outside_deployed_environments() {
        let mut config = WebConfig::for_tests();
        assert!(!config.cookies_secure());
        config.environment = Environment::Production;
        assert!(config.cookies_secure());
    }
}
