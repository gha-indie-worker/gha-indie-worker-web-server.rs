#![forbid(unsafe_code)]
//! Liveness and readiness.
//!
//! `/healthz` answers as long as the process is running: it is the probe Cloud Run restarts on,
//! and a probe that fails because a *dependency* is unwell turns one sick dependency into a
//! restart loop. `/readyz` is the one that reports dependencies, so a load balancer can stop
//! sending traffic without anything being killed.
//!
//! Both are plain text, both are outside the host-routing gate — a probe reaches this service
//! through the container's own address, with no `Host` header of ours — and neither says anything
//! a stranger could use.

use axum::extract::State;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::state::{now_seconds, AppState};

/// `GET /healthz`.
pub async fn healthz(State(state): State<AppState>) -> Response {
    let uptime = now_seconds().saturating_sub(state.started_at);
    text(
        StatusCode::OK,
        format!("ok\nservice=gha-indie-worker-web-server\nuptime={uptime}s\n"),
    )
}

/// `GET /readyz`.
///
/// Reports which dependencies are configured. It is deliberately not a health *check* — this tier
/// does not open a database connection to answer a probe, because a probe that does work is a
/// denial-of-service amplifier.
pub async fn readyz(State(state): State<AppState>) -> Response {
    let warnings = state.config.warnings();
    let assets = crate::assets::assets().warnings();
    let degraded = !warnings.is_empty() || !assets.is_empty();
    let mut body = String::from(if degraded { "degraded\n" } else { "ready\n" });
    for warning in warnings.iter().chain(assets.iter()) {
        body.push_str("- ");
        body.push_str(warning);
        body.push('\n');
    }
    // Degraded is still serving: the development stub answers requests, and a readiness probe that
    // fails closed on a missing optional dependency takes the site down to report a warning.
    text(StatusCode::OK, body)
}

fn text(status: StatusCode, body: String) -> Response {
    let mut response = (status, body).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}
