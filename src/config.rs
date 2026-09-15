#![forbid(unsafe_code)]

#[derive(Clone, Debug)]
pub struct WebConfig {
    pub bind: String,
    pub api_http_base: Option<String>,
    pub database_url: Option<String>,
}

impl WebConfig {
    pub fn from_env() -> Self {
        Self {
            bind: std::env::var("GHA_INDIE_WORKER_WEB_BIND")
                .unwrap_or_else(|_| "127.0.0.1:8081".into()),
            api_http_base: std::env::var("GHA_INDIE_WORKER_API_HTTP_BASE").ok(),
            database_url: std::env::var("GHA_INDIE_WORKER_DATABASE_URL").ok(),
        }
    }
}
