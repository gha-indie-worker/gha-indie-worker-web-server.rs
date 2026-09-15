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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{to_bytes, Body},
        http::{header, Method, Request, StatusCode},
    };
    use tower::ServiceExt;

    async fn request(method: Method, path: &str) -> (StatusCode, String, Vec<u8>) {
        let response = router()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("router response");
        let status = response.status();
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let body = to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("response body")
            .to_vec();
        (status, content_type, body)
    }

    #[tokio::test]
    async fn home_surface_is_html_and_contains_product_identity() {
        let (status, content_type, body) = request(Method::GET, "/").await;
        assert_eq!(status, StatusCode::OK);
        assert!(content_type.starts_with("text/html"));
        let text = String::from_utf8(body).expect("html body");
        assert!(text.contains("GHA Indie Worker"));
        assert!(text.contains("No React"));
    }

    #[tokio::test]
    async fn health_and_ready_are_empty_success_responses() {
        for path in ["/healthz", "/readyz"] {
            let (status, _, body) = request(Method::GET, path).await;
            assert_eq!(status, StatusCode::NO_CONTENT);
            assert!(body.is_empty());
        }
    }

    #[tokio::test]
    async fn router_fails_closed_for_unknown_routes_and_wrong_methods() {
        let (missing, _, _) = request(Method::GET, "/does-not-exist").await;
        let (wrong_method, _, _) = request(Method::POST, "/").await;
        assert_eq!(missing, StatusCode::NOT_FOUND);
        assert_eq!(wrong_method, StatusCode::METHOD_NOT_ALLOWED);
    }
}
