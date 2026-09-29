#![forbid(unsafe_code)]

use gha_indie_worker_web_server::{config::WebConfig, error::WebError, server};

fn main() -> Result<(), WebError> {
    let cfg = WebConfig::from_env();
    server::run(&cfg)
}
