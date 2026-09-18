#![forbid(unsafe_code)]

use gha_indie_worker_web_server::{config::WebConfig, error::WebError, flags, server};
use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let environment = flags::resolve().map_err(WebError::ConfigurationResolution)?;
    let cfg = WebConfig::from_env_map(&environment);
    server::run(&cfg).await
}
