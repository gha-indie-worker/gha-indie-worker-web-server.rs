#![forbid(unsafe_code)]

use gha_indie_worker_web_server::{config::WebConfig, server};

fn main() -> std::io::Result<()> {
    let cfg = WebConfig::from_env();
    server::run(&cfg)
}
