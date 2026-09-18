#![forbid(unsafe_code)]

#[allow(clippy::match_like_matches_macro)]
#[rustfmt::skip]
#[path = "../generated/rust/runtime.rs"]
mod env_runtime;

use crate::env_map::{value, EnvMap};

const DATABASE_URL_ENV: &str = "GHA_INDIE_WORKER_DATABASE_URL";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebConfig {
    pub bind: String,
    pub api_http_base: Option<String>,
    pub database_url: Option<String>,
}

impl WebConfig {
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        // Public argv/env configuration remains generated from `.cli-flags.toml`.
        // Database URLs may contain credentials, so they are deliberately not
        // public CLI flags and are read only from the injected secret/env boundary.
        let values = env_runtime::load_from(|key| lookup(key));
        let database_url = lookup(DATABASE_URL_ENV).filter(|value| !value.is_empty());
        Self {
            bind: values
                .gha_indie_worker_web_bind
                .unwrap_or_else(|| "127.0.0.1:8081".into()),
            api_http_base: values.gha_indie_worker_api_http_base,
            database_url,
        }
    }

    /// Read configuration out of an immutable environment snapshot: the
    /// process environment copied at the boundary, with CLI flag overrides
    /// overlaid. Nothing here reads or writes the process environment, which
    /// is the point of `env_map` - see `src/flags.rs`.
    pub fn from_env_map(env: &EnvMap) -> Self {
        Self::from_lookup(|key| value(env, key).map(str::to_owned))
    }

    pub fn from_env() -> Self {
        Self::from_lookup(|key| std::env::var(key).ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn from_pairs(values: &[(&str, &str)]) -> WebConfig {
        let values = values
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect::<BTreeMap<_, _>>();
        WebConfig::from_lookup(|key| values.get(key).cloned())
    }

    #[test]
    fn generated_public_values_and_secret_database_env_drive_web_config() {
        let config = from_pairs(&[
            ("GHA_INDIE_WORKER_WEB_BIND", "127.0.0.1:18081"),
            ("GHA_INDIE_WORKER_API_HTTP_BASE", "https://api.example.test"),
            (
                DATABASE_URL_ENV,
                "postgres://synthetic:secret@invalid.example/db",
            ),
        ]);
        assert_eq!(
            config,
            WebConfig {
                bind: "127.0.0.1:18081".into(),
                api_http_base: Some("https://api.example.test".into()),
                database_url: Some("postgres://synthetic:secret@invalid.example/db".into()),
            }
        );
    }

    #[test]
    fn empty_values_follow_runtime_semantics() {
        let config = from_pairs(&[
            ("GHA_INDIE_WORKER_WEB_BIND", ""),
            ("GHA_INDIE_WORKER_API_HTTP_BASE", ""),
            (DATABASE_URL_ENV, ""),
        ]);
        assert_eq!(config.bind, "127.0.0.1:8081");
        assert_eq!(config.api_http_base, None);
        assert_eq!(config.database_url, None);
    }

    #[test]
    fn env_map_snapshot_drives_web_config_without_touching_process_env() {
        let before = std::env::var_os("GHA_INDIE_WORKER_WEB_BIND");
        let env = EnvMap::from([
            (
                "GHA_INDIE_WORKER_WEB_BIND".to_owned(),
                "  127.0.0.1:19090  ".to_owned(),
            ),
            ("GHA_INDIE_WORKER_API_HTTP_BASE".to_owned(), "".to_owned()),
        ]);
        let config = WebConfig::from_env_map(&env);
        assert_eq!(config.bind, "127.0.0.1:19090");
        assert_eq!(config.api_http_base, None);
        assert_eq!(config.database_url, None);
        assert_eq!(std::env::var_os("GHA_INDIE_WORKER_WEB_BIND"), before);
    }

    #[test]
    fn secret_database_url_is_not_part_of_generated_public_runtime() {
        let generated = include_str!("../generated/rust/runtime.rs");
        assert!(!generated.contains(DATABASE_URL_ENV));
        let public_contract = include_str!("../.cli-flags.toml");
        assert!(!public_contract.contains("gha-indie-worker-database-url"));
    }
}
