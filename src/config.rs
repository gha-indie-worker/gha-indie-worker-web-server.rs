#![forbid(unsafe_code)]

#[path = "../generated/rust/runtime.rs"]
mod env_runtime;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebConfig {
    pub bind: String,
    pub api_http_base: Option<String>,
    pub database_url: Option<String>,
}

impl WebConfig {
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        let values = env_runtime::load_from(lookup);
        Self {
            bind: values
                .gha_indie_worker_web_bind
                .unwrap_or_else(|| "127.0.0.1:8081".into()),
            api_http_base: values.gha_indie_worker_api_http_base,
            database_url: values.gha_indie_worker_database_url,
        }
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
    fn generated_runtime_values_drive_web_config() {
        let config = from_pairs(&[
            ("GHA_INDIE_WORKER_WEB_BIND", "127.0.0.1:18081"),
            ("GHA_INDIE_WORKER_API_HTTP_BASE", "https://api.example.test"),
            ("GHA_INDIE_WORKER_DATABASE_URL", "postgres://synthetic.invalid/db"),
        ]);
        assert_eq!(
            config,
            WebConfig {
                bind: "127.0.0.1:18081".into(),
                api_http_base: Some("https://api.example.test".into()),
                database_url: Some("postgres://synthetic.invalid/db".into()),
            }
        );
    }

    #[test]
    fn empty_values_follow_generated_runtime_semantics() {
        let config = from_pairs(&[
            ("GHA_INDIE_WORKER_WEB_BIND", ""),
            ("GHA_INDIE_WORKER_API_HTTP_BASE", ""),
            ("GHA_INDIE_WORKER_DATABASE_URL", ""),
        ]);
        assert_eq!(config.bind, "127.0.0.1:8081");
        assert_eq!(config.api_http_base, None);
        assert_eq!(config.database_url, None);
    }
}
