#![forbid(unsafe_code)]

use gha_indie_worker_web_server::{config::WebConfig, server};
use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let cfg = WebConfig::from_env();
    server::run(&cfg).await
}
