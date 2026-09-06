#![forbid(unsafe_code)]

use gha_indie_worker_web_server::{config::WebConfig, server, SERVICE};

#[tokio::main]
async fn main() -> Result<(), server::BootError> {
    // JSON logs in a deployment, human-readable on a terminal. `ores-otel` reads the same
    // `ORES_OTEL_*` variables the middleware does and is wired in at the middleware layer, so
    // there is nothing to configure here beyond the local subscriber.
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,tower_http=warn"));
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    if std::io::IsTerminal::is_terminal(&std::io::stderr()) {
        builder.with_writer(std::io::stderr).init();
    } else {
        builder.json().with_writer(std::io::stderr).init();
    }

    tracing::info!(service.name = SERVICE, "starting");
    server::run(WebConfig::from_env()).await
}
