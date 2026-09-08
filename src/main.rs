#![forbid(unsafe_code)]

//! Process entry point.
//!
//! Order matters: telemetry first, so a configuration failure is reported as a
//! structured log rather than a bare `eprintln!`; then configuration; then the
//! server. A configuration error exits non-zero **before** anything binds — the
//! fleet contract is that a misconfigured service fails to start rather than
//! serving in a degraded posture nobody noticed.

use gha_indie_worker_web_server::{config::WebConfig, server, telemetry};

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let guard = telemetry::init();
    let logger = guard.logger();

    let config = WebConfig::from_env();
    if let Err(error) = config.validate() {
        tracing::error!(error.kind = "invalid_config", error.detail = %error, "refusing to start");
        return std::process::ExitCode::from(78); // EX_CONFIG
    }

    match server::run(config, Some(logger)).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(error.kind = "server_stopped", error.detail = %error, "server stopped");
            std::process::ExitCode::FAILURE
        }
    }
}
