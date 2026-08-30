#![forbid(unsafe_code)]

use gha_indie_worker_web_server::{config::WebConfig, error::WebError, flags, server};

fn main() -> Result<(), WebError> {
    let environment = flags::resolve().map_err(WebError::ConfigurationResolution)?;
    let cfg = WebConfig::from_map(&environment);
    server::run(&cfg)
}
