#![forbid(unsafe_code)]

//! Ores structured logging bridged into this service's JSON tracing stream.
//!
//! Same shape as `canonical-api-server.rs/src/telemetry.rs` — one subscriber,
//! one ores-otel [`Logger`] whose records are re-emitted through `tracing` at the
//! matching level. The bridge never attaches credentials, cookies, URLs with
//! query strings, request bodies, identity values, or upstream response bodies.
//!
//! The [`Logger`] returned by [`TelemetryGuard::logger`] is handed to
//! ores-middleware so every request log carries the same request/trace ids as
//! the rest of the fleet.

use std::sync::Arc;

use next_loggers::{json, JsonObject, LogLevel, LogRecord, Logger, LoggerError, Options, Transport};
use tracing_subscriber::EnvFilter;

const SERVICE_NAME: &str = crate::SERVICE_NAME;
const SERVICE_NAMESPACE: &str = crate::SERVICE_NAMESPACE;

pub struct TelemetryGuard {
    ores_logger: Logger,
}

impl TelemetryGuard {
    /// A clone of the process logger, for ores-middleware and for modules that
    /// want `info_context`/`warn_context` inside a request task.
    #[must_use]
    pub fn logger(&self) -> Logger {
        self.ores_logger.clone()
    }
}

impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        if self.ores_logger.close().is_err() {
            eprintln!("telemetry: Ores logger shutdown failed; final records may be incomplete");
        }
    }
}

/// Installs the tracing subscriber and the ores-otel bridge. Call once, keep the
/// guard alive for the process lifetime.
#[must_use]
pub fn init() -> TelemetryGuard {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .json()
        .with_ansi(false)
        .with_target(true)
        .init();

    let ores_logger = Logger::new(Options {
        app_name: SERVICE_NAME.to_string(),
        name: Some("web".to_string()),
        console: false,
        transports: vec![Arc::new(TracingBridgeTransport)],
        ..Options::default()
    });
    let _ = ores_logger
        .info(vec![json!("telemetry initialized")])
        .add_fields(JsonObject::from_iter([
            ("service.name".to_string(), json!(SERVICE_NAME)),
            ("service.namespace".to_string(), json!(SERVICE_NAMESPACE)),
            ("log.destination".to_string(), json!("tracing-bridge")),
        ]))
        .send();
    tracing::info!(
        service.name = SERVICE_NAME,
        service.namespace = SERVICE_NAMESPACE,
        log.format = "json",
        log.destination = "stderr",
        "telemetry initialized"
    );

    TelemetryGuard { ores_logger }
}

struct TracingBridgeTransport;

impl Transport for TracingBridgeTransport {
    fn write(&self, record: &LogRecord) -> Result<(), LoggerError> {
        let encoded = record.to_json()?;
        match record.level {
            LogLevel::Trace => tracing::trace!(ores.record = %encoded, "Ores structured log"),
            LogLevel::Debug => tracing::debug!(ores.record = %encoded, "Ores structured log"),
            LogLevel::Info => tracing::info!(ores.record = %encoded, "Ores structured log"),
            LogLevel::Warn => tracing::warn!(ores.record = %encoded, "Ores structured log"),
            LogLevel::Error => tracing::error!(ores.record = %encoded, "Ores structured log"),
            LogLevel::Fatal => tracing::error!(ores.record = %encoded, "Ores fatal structured log"),
        }
        Ok(())
    }

    fn is_open_telemetry(&self) -> bool {
        true
    }
}
