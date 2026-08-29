#![forbid(unsafe_code)]

use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct WebConfig {
    pub bind: String,
    pub api_http_base: Option<String>,
    pub database_url: Option<String>,
}

impl WebConfig {
    pub fn from_env() -> Self {
        Self::from_map(&std::env::vars().collect())
    }

    pub fn from_map(environment: &BTreeMap<String, String>) -> Self {
        Self {
            bind: environment
                .get("GHA_INDIE_WORKER_WEB_BIND")
                .cloned()
                .unwrap_or_else(|| "127.0.0.1:8081".into()),
            api_http_base: environment.get("GHA_INDIE_WORKER_API_HTTP_BASE").cloned(),
            database_url: environment.get("GHA_INDIE_WORKER_DATABASE_URL").cloned(),
        }
    }
}
