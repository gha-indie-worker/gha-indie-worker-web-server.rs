#![forbid(unsafe_code)]

//! Process entry point.
//!
//! Order matters: telemetry first, so a configuration failure is reported as a
//! structured log rather than a bare `eprintln!`; then configuration; then the
//! server. A configuration error exits non-zero **before** anything binds — the
//! fleet contract is that a misconfigured service fails to start rather than
//! serving in a degraded posture nobody noticed.

use std::collections::BTreeMap;

use gha_indie_worker_web_server::{config::WebConfig, flags, server, telemetry};

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let guard = telemetry::init();
    let logger = guard.logger();

    // flags-2-env is the fail-closed authority for argv and the keys declared in
    // `.cli-flags.toml`: an unknown or malformed option stops the process here,
    // without echoing its value. Its resolved values are laid over the process
    // environment, so anything the deployment injects that is not a CLI surface
    // (PORT, ORES_MIDDLEWARE_ENV, …) still reaches the configuration.
    let resolved = match flags::resolve() {
        Ok(resolved) => resolved,
        Err(error) => {
            tracing::error!(error.kind = "invalid_flags", error.detail = %error, "refusing to start");
            return std::process::ExitCode::from(78); // EX_CONFIG
        }
    };
    let mut environment: BTreeMap<String, String> = std::env::vars().collect();
    environment.extend(resolved);

    let config = WebConfig::from_map(&environment);
    if let Err(error) = config.validate() {
        tracing::error!(error.kind = "invalid_config", error.detail = %error, "refusing to start");
        return std::process::ExitCode::from(78); // EX_CONFIG
    }

    match server::run(config, Some(logger)).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error @ server::ServerError::Plan(_)) => {
            tracing::error!(error.kind = "invalid_config", error.detail = %error, "refusing to start");
            std::process::ExitCode::from(78)
        }
        Err(error) => {
            tracing::error!(error.kind = "server_stopped", error.detail = %error, "server stopped");
            std::process::ExitCode::FAILURE
        }
    }
}
