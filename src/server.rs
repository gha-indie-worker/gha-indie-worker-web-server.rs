#![forbid(unsafe_code)]

//! Composition and process lifecycle.
//!
//! The router is assembled in one place, in this order (outermost first):
//!
//! 1. **ores-middleware** — the fleet pipeline, with this service's auth and
//!    rate-limit hooks ([`crate::middleware::install`]);
//! 2. **[`crate::middleware::request_context`]** — surface resolution, CSP
//!    nonce, CSRF, actor;
//! 3. **`/assets`** — the static mount (self-hosted htmx, the ores-web-loader
//!    module, immutable release assets);
//! 4. **host dispatch** — the per-surface routers.
//!
//! [`build_router`] stops at step 2 so tests can drive the whole HTTP surface
//! with `tower::ServiceExt::oneshot` without ores-middleware reading the
//! environment. [`run`] adds step 1 and binds the listener.

use std::net::SocketAddr;

use axum::Router;
use next_loggers::Logger;
use thiserror::Error;
use tower_http::services::ServeDir;

use crate::config::WebConfig;
use crate::hosts;
use crate::state::AppState;

#[derive(Debug, Error)]
pub enum ServerError {
    #[error("configuration is invalid: {0}")]
    Config(#[from] crate::config::ConfigError),
    #[error("the middleware pipeline could not be built: {0}")]
    Middleware(String),
    #[error("could not bind {bind}")]
    Bind { bind: String },
    #[error("the server stopped unexpectedly")]
    Serve,
}

/// The router without ores-middleware: host dispatch, static assets, and the
/// request-context layer. This is what the HTTP tests exercise.
#[must_use]
pub fn build_router(state: &AppState) -> Router {
    let assets = ServeDir::new(state.config.assets_dir.clone()).append_index_html_on_directories(false);

    hosts::dispatch_router(state)
        .merge(crate::releases::router(state.clone()))
        .nest_service("/assets", assets)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::middleware::request_context,
        ))
}

/// The full router, with the fleet middleware installed around it.
pub fn build_service(state: &AppState, logger: Option<Logger>) -> Result<Router, ServerError> {
    crate::middleware::install(build_router(state), state, logger)
        .map_err(|error| ServerError::Middleware(error.to_string()))
}

/// Builds the state, binds, serves, and shuts down gracefully.
pub async fn run(config: WebConfig, logger: Option<Logger>) -> Result<(), ServerError> {
    config.validate()?;
    let bind = config.bind.clone();
    let state = AppState::build(config).await;

    #[cfg(feature = "tcp-transport")]
    spawn_tcp_avenue(&state);

    let app = build_service(&state, logger)?;

    let listener = tokio::net::TcpListener::bind(&bind)
        .await
        .map_err(|_| ServerError::Bind { bind: bind.clone() })?;
    tracing::info!(
        service.name = crate::SERVICE_NAME,
        server.address = %bind,
        surfaces = "app,user,org,m",
        "web server listening"
    );

    // ConnectInfo is what makes the trusted-proxy check possible: without the
    // peer address, `X-Forwarded-Host` could never be honoured safely.
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|_| ServerError::Serve)
}

/// SIGTERM (Cloud Run's stop signal) or Ctrl-C.
pub async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
    tracing::info!(service.name = crate::SERVICE_NAME, "shutdown signal received");
}

/// The stateful TCP avenue, when one is configured.
#[cfg(feature = "tcp-transport")]
fn spawn_tcp_avenue(state: &AppState) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let Some(bind) = state.config.tcp_bind.clone() else {
        return;
    };
    let base_domain = state.config.base_domain.clone();
    let read_source = state.repo.read_source().as_str().to_owned();

    tokio::spawn(async move {
        let listener = match tokio::net::TcpListener::bind(&bind).await {
            Ok(listener) => listener,
            Err(_) => {
                tracing::warn!(avenue = "tcp", server.address = %bind, "tcp avenue could not bind");
                return;
            }
        };
        tracing::info!(avenue = "tcp", server.address = %bind, "tcp avenue listening");

        loop {
            let Ok((mut socket, _peer)) = listener.accept().await else {
                continue;
            };
            let base_domain = base_domain.clone();
            let read_source = read_source.clone();
            tokio::spawn(async move {
                let mut header = [0u8; crate::transport::tcp::LENGTH_PREFIX_BYTES];
                if socket.read_exact(&mut header).await.is_err() {
                    return;
                }
                let Ok(length) = crate::transport::tcp::frame_length(&header) else {
                    return;
                };
                let mut body = vec![0u8; length];
                if socket.read_exact(&mut body).await.is_err() {
                    return;
                }
                let response = match serde_json::from_slice::<crate::transport::tcp::TcpRequest>(&body) {
                    Ok(request) => crate::transport::tcp::handle(&request, &base_domain, &read_source),
                    Err(_) => crate::transport::tcp::TcpResponse::Error {
                        code: "malformed_frame".to_owned(),
                    },
                };
                if let Ok(frame) = crate::transport::tcp::encode(&response) {
                    let _ = socket.write_all(&frame).await;
                }
                let _ = socket.shutdown().await;
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_test_router_builds_without_touching_the_environment() {
        let state = AppState::for_tests(WebConfig::for_tests());
        let _router = build_router(&state);
    }

    #[test]
    fn server_errors_never_carry_a_connection_string() {
        let error = ServerError::Bind {
            bind: "127.0.0.1:8081".into(),
        };
        assert_eq!(error.to_string(), "could not bind 127.0.0.1:8081");
        assert!(!ServerError::Serve.to_string().contains("postgres"));
    }
}
