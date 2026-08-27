#![forbid(unsafe_code)]

use crate::env_map::{value, EnvMap};

#[derive(Clone, Debug)]
pub struct WebConfig {
    pub bind: String,
    pub api_http_base: Option<String>,
    pub database_url: Option<String>,
}

impl WebConfig {
    pub fn from_env_map(env: &EnvMap) -> Self {
        Self {
            bind: value(env, "GHA_INDIE_WORKER_WEB_BIND")
                .unwrap_or("127.0.0.1:8081")
                .to_owned(),
            api_http_base: value(env, "GHA_INDIE_WORKER_API_HTTP_BASE").map(str::to_owned),
            database_url: value(env, "GHA_INDIE_WORKER_DATABASE_URL").map(str::to_owned),
        }
    }

    pub fn from_env() -> Self {
        Self::from_env_map(&std::env::vars().collect())
    }
}
