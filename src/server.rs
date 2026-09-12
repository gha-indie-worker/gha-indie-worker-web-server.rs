#![forbid(unsafe_code)]
//! Boot: validate the startup plan, build the router, wrap it in the ORES middleware stack,
//! listen, and shut down cleanly.
//!
//! [`startup_plan`] is the pure effect boundary. It validates immutable configuration and derives
//! non-sensitive backend capabilities before anything binds; database connection strings never
//! enter the printable plan.
//!
//! The middleware install is **fail-closed**. `ores_middleware` is what enforces the trusted-proxy
//! boundary, the body limit and the rate limit, and those are configured by the same environment
//! variables `gcp/cloudrun/services.tf` sets. A process that could not build that stack has not
//! got a degraded security posture, it has none, so it refuses to start rather than quietly
//! serving without it.

use std::net::SocketAddr;

use axum::Router;

use crate::config::WebConfig;
use crate::error::WebError;
use crate::{assets, pages, state::AppState, SERVICE};

/// Anything that stops the server starting.
pub type BootError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendCapability {
    DirectReadOnlyDatabase,
    StatelessHttp,
    DurableNats,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartupPlan {
    pub bind: String,
    pub capabilities: Vec<BackendCapability>,
}

/// Validate configuration and derive the capabilities this process will use.
///
/// # Errors
/// [`WebError::InvalidConfiguration`] naming the key when a configured value is blank.
pub fn startup_plan(config: &WebConfig) -> Result<StartupPlan, WebError> {
    let bind = non_empty("GHA_INDIE_WORKER_WEB_BIND", &config.bind)?;
    let optional_capabilities = [
        config.database_url.expose().map(|value| {
            (
                BackendCapability::DirectReadOnlyDatabase,
                "GHA_INDIE_WORKER_DATABASE_URL",
                value,
            )
        }),
        config.api_http_base.as_deref().map(|value| {
            (
                BackendCapability::StatelessHttp,
                "GHA_INDIE_WORKER_API_HTTP_BASE",
                value,
            )
        }),
        config.nats_url.as_deref().map(|value| {
            (
                BackendCapability::DurableNats,
                "GHA_INDIE_WORKER_NATS_URL",
                value,
            )
        }),
    ];
    let capabilities = optional_capabilities
        .into_iter()
        .flatten()
        .map(|(capability, field, value)| non_empty(field, value).map(|_| capability))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(StartupPlan { bind, capabilities })
}

fn non_empty(field: &'static str, value: &str) -> Result<String, WebError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(WebError::InvalidConfiguration(field));
    }
    Ok(value.to_owned())
}

/// Build the fully decorated router for a given configuration. Exposed so router-level tests can
/// drive the application with `tower::ServiceExt::oneshot` and never bind a socket.
///
/// # Errors
/// [`BootError`] when the ORES middleware stack cannot be built from the environment.
pub fn build(config: WebConfig) -> Result<Router, BootError> {
    let state = AppState::new(config);
    build_with_state(state)
}

/// As [`build`], for a state a test has already prepared.
///
/// # Errors
/// [`BootError`] when the ORES middleware stack cannot be built from the environment.
pub fn build_with_state(state: AppState) -> Result<Router, BootError> {
    let router = pages::router(state);
    let router = ores_middleware::frameworks::axum::install_from_env(router, SERVICE)?;
    Ok(router)
}

/// Validate the startup plan, then bind and serve until a shutdown signal arrives.
///
/// # Errors
/// [`BootError`] for an invalid plan, a middleware, address or listener failure.
pub async fn run(config: WebConfig) -> Result<(), BootError> {
    let plan = startup_plan(&config)?;
    for warning in config.warnings() {
        tracing::warn!(%warning, "configuration");
    }
    for warning in assets::assets().warnings() {
        tracing::warn!(%warning, "assets");
    }

    let apex = config.apex.clone();
    let address: SocketAddr = plan.bind.parse()?;
    let app = build(config)?;

    let listener = tokio::net::TcpListener::bind(address).await?;
    tracing::info!(
        %address,
        %apex,
        service.name = SERVICE,
        capabilities = ?plan.capabilities,
        htmx = assets::assets().htmx_available(),
        "web server listening"
    );
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

/// Wait for Ctrl-C or `SIGTERM`. Cloud Run sends `SIGTERM` and then waits, so honouring it is what
/// makes a deploy drain rather than drop connections — including open log-tail sockets.
pub async fn shutdown_signal() {
    let interrupt = async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::error!(%error, "could not install the Ctrl-C handler");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => tracing::error!(%error, "could not install the SIGTERM handler"),
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = interrupt => {},
        () = terminate => {},
    }
    tracing::info!("shutting down");
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{startup_plan, BackendCapability, StartupPlan};
    use crate::{config::WebConfig, error::WebError};

    fn config(entries: &[(&str, &str)]) -> WebConfig {
        WebConfig::from_map(
            &entries
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect::<BTreeMap<_, _>>(),
        )
    }

    #[test]
    fn startup_plan_derives_capabilities_without_retaining_the_database_url() {
        let config = config(&[
            ("GHA_INDIE_WORKER_WEB_BIND", " 127.0.0.1:8081 "),
            ("GHA_INDIE_WORKER_API_HTTP_BASE", "http://api:8080"),
            ("GHA_INDIE_WORKER_DATABASE_URL", "postgres://sensitive-value"),
            (
                "GHA_INDIE_WORKER_NATS_URL",
                "nats://worker:credential@nats.internal:4222",
            ),
        ]);

        let plan = startup_plan(&config).expect("valid web startup plan");

        assert_eq!(
            plan,
            StartupPlan {
                bind: "127.0.0.1:8081".into(),
                capabilities: vec![
                    BackendCapability::DirectReadOnlyDatabase,
                    BackendCapability::StatelessHttp,
                    BackendCapability::DurableNats,
                ],
            }
        );
        assert!(!format!("{plan:?}").contains("sensitive-value"));
        assert!(!format!("{plan:?}").contains("credential"));
        assert!(!format!("{config:?}").contains("sensitive-value"));
    }

    #[test]
    fn startup_plan_rejects_blank_optional_configuration() {
        let error = startup_plan(&config(&[
            ("GHA_INDIE_WORKER_WEB_BIND", "127.0.0.1:8081"),
            ("GHA_INDIE_WORKER_API_HTTP_BASE", "  "),
        ]))
        .expect_err("blank API base must fail closed");

        assert!(matches!(
            error,
            WebError::InvalidConfiguration("GHA_INDIE_WORKER_API_HTTP_BASE")
        ));
    }
}
