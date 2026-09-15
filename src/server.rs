#![forbid(unsafe_code)]

use crate::config::WebConfig;
use crate::pages;
use axum::{http::StatusCode, response::Html, routing::get, Router};
use std::error::Error;

pub fn router() -> Router {
    Router::new()
        .route("/", get(|| async { Html(pages::home::markup()) }))
        .route("/healthz", get(|| async { StatusCode::NO_CONTENT }))
        .route("/readyz", get(|| async { StatusCode::NO_CONTENT }))
}

pub async fn run(config: &WebConfig) -> Result<(), Box<dyn Error>> {
    let listener = tokio::net::TcpListener::bind(&config.bind).await?;
    eprintln!(
        "gha-indie-worker-web-server listening on {}",
        listener.local_addr()?
    );
    axum::serve(listener, router()).await?;
    Ok(())
}
