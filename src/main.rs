#![forbid(unsafe_code)]

use gha_indie_worker_web_server::{config::WebConfig, flags, server};
use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let env = match flags::apply_cli_flags() {
        Ok(env) => env,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    };
    let cfg = WebConfig::from_env_map(&env);
    server::run(&cfg).await
}
