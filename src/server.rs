#![forbid(unsafe_code)]
//! Boot: build the router, wrap it in the ORES middleware stack, listen, and shut down cleanly.
//!
//! The middleware install is **fail-closed**. `ores_middleware` is what enforces the trusted-proxy
//! boundary, the body limit and the rate limit, and those are configured by the same environment
//! variables `gcp/cloudrun/services.tf` sets. A process that could not build that stack has not
//! got a degraded security posture, it has none, so it refuses to start rather than quietly
//! serving without it.

use std::net::SocketAddr;

use axum::Router;

use crate::config::WebConfig;
use crate::{assets, pages, state::AppState, SERVICE};

/// Anything that stops the server starting.
pub type BootError = Box<dyn std::error::Error + Send + Sync>;

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

/// Bind and serve until a shutdown signal arrives.
///
/// # Errors
/// [`BootError`] for a middleware, address or listener failure.
pub async fn run(config: WebConfig) -> Result<(), BootError> {
    for warning in config.warnings() {
        tracing::warn!(%warning, "configuration");
    }
    for warning in assets::assets().warnings() {
        tracing::warn!(%warning, "assets");
    }

    let bind = config.bind.clone();
    let apex = config.apex.clone();
    let address: SocketAddr = bind.parse()?;
    let app = build(config)?;

    let listener = tokio::net::TcpListener::bind(address).await?;
    tracing::info!(
        %address,
        %apex,
        service.name = SERVICE,
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
