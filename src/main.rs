#![forbid(unsafe_code)]
#![allow(clippy::needless_return)]

use gha_indie_worker_web_server::{config::WebConfig, server};

fn main() -> std::io::Result<()> {
    let cfg = WebConfig::from_env();
    return server::run(&cfg);
}
