#![forbid(unsafe_code)]

//! The shared application state.
//!
//! Everything in here is cheap to clone (`Arc` or a `reqwest::Client`, which is
//! itself an `Arc` internally), because axum clones the state once per request.
//! Nothing here is mutable: request-scoped values live in
//! [`crate::middleware::RequestCtx`].

use std::sync::Arc;

use crate::auth::AuthContext;
use crate::chat::ChatService;
use crate::config::WebConfig;
use crate::csrf::CsrfKey;
use crate::data::{api_client::default_http_client, ApiClient, Repo};
use crate::hosts::HostPolicy;
use crate::session::SessionCodec;

/// Used when `GHA_INDIE_WORKER_SESSION_SECRET` is unset outside production.
/// [`WebConfig::validate`] refuses to let this reach a deployed environment.
const DEVELOPMENT_SESSION_SECRET: &str = "gha-indie-worker-web-development-only";

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<WebConfig>,
    pub hosts: Arc<HostPolicy>,
    pub auth: Arc<AuthContext>,
    pub sessions: Arc<SessionCodec>,
    pub csrf: Arc<CsrfKey>,
    pub repo: Repo,
    pub chat: Arc<ChatService>,
    pub http: reqwest::Client,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AppState")
            .field("surface_base", &self.config.base_domain)
            .field("environment", &self.config.environment.as_str())
            .field("repo", &self.repo)
            .finish_non_exhaustive()
    }
}

impl AppState {
    /// Builds the state, opening the canonical read pool when one is configured.
    ///
    /// A database that will not open is **not** fatal: the api-server can answer
    /// every read, so the site stays up and degrades to one avenue instead of
    /// refusing to boot.
    pub async fn build(config: WebConfig) -> Self {
        let http = default_http_client();
        let api = Arc::new(ApiClient::new(config.api_http_base.clone(), http.clone()));

        #[cfg(feature = "db")]
        let repo = {
            let pool = match config.database_url_canonical.as_deref() {
                Some(url) => match crate::data::db::ReadPool::connect(url).await {
                    Ok(pool) => Some(Arc::new(pool)),
                    Err(_) => {
                        tracing::warn!(
                            avenue = "database",
                            "canonical read pool unavailable; reads will use the api-server"
                        );
                        None
                    }
                },
                None => None,
            };
            Repo::new(Arc::clone(&api), pool)
        };
        #[cfg(not(feature = "db"))]
        let repo = Repo::new(Arc::clone(&api));

        Self::assemble(config, http, api, repo)
    }

    /// A state with no I/O for unit and HTTP tests.
    #[must_use]
    pub fn for_tests(config: WebConfig) -> Self {
        let http = default_http_client();
        let api = Arc::new(ApiClient::new(config.api_http_base.clone(), http.clone()));
        #[cfg(feature = "db")]
        let repo = Repo::new(Arc::clone(&api), None);
        #[cfg(not(feature = "db"))]
        let repo = Repo::new(Arc::clone(&api));
        Self::assemble(config, http, api, repo)
    }

    fn assemble(config: WebConfig, http: reqwest::Client, api: Arc<ApiClient>, repo: Repo) -> Self {
        let secret = config
            .session_secret
            .clone()
            .unwrap_or_else(|| DEVELOPMENT_SESSION_SECRET.to_owned());
        let hosts = Arc::new(HostPolicy::new(
            config.base_domain.clone(),
            &config.trusted_proxy_cidrs,
            config.dev_surface,
        ));
        let sessions = Arc::new(SessionCodec::new(
            &secret,
            config.session_cookie_name.clone(),
            config.cookies_secure(),
            config.session_ttl_seconds,
        ));
        let csrf = Arc::new(CsrfKey::new(
            &secret,
            config.csrf_cookie_name.clone(),
            config.cookies_secure(),
        ));
        let auth = Arc::new(AuthContext::from_config(&config, http.clone()));
        let chat = Arc::new(ChatService::new(Arc::clone(&api), config.chat_enabled));
        Self {
            config: Arc::new(config),
            hosts,
            auth,
            sessions,
            csrf,
            repo,
            chat,
            http,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hosts::Surface;

    #[test]
    fn a_test_state_needs_no_network_and_no_database() {
        let state = AppState::for_tests(WebConfig::for_tests());
        assert_eq!(state.repo.read_source(), crate::data::ReadSource::ApiServer);
        assert_eq!(state.hosts.surface_for_host("org.indiebuild.dev"), Surface::Org);
        assert!(!state.auth.is_configured());
    }

    #[test]
    fn cookies_are_insecure_only_in_development_and_test() {
        let state = AppState::for_tests(WebConfig::for_tests());
        let cookie = state
            .sessions
            .set_cookie("v")
            .expect("header value")
            .to_str()
            .expect("ascii")
            .to_owned();
        assert!(!cookie.contains("Secure"));

        let mut config = WebConfig::for_tests();
        config.environment = crate::config::Environment::Production;
        let deployed = AppState::for_tests(config);
        let cookie = deployed
            .sessions
            .set_cookie("v")
            .expect("header value")
            .to_str()
            .expect("ascii")
            .to_owned();
        assert!(cookie.contains("Secure"));
    }

    #[test]
    fn the_debug_view_hides_secrets() {
        let text = format!("{:?}", AppState::for_tests(WebConfig::for_tests()));
        assert!(!text.contains("test-session-secret-not-for-production"));
        assert!(!text.contains("secret"));
    }
}
