#![forbid(unsafe_code)]

use crate::config::WebConfig;
use crate::error::WebError;
use crate::pages;
use axum::{http::StatusCode, response::Html, routing::get, Router};
use std::error::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendCapability {
    DirectReadOnlyDatabase,
    StatelessHttp,
}

/// What this process will do once it starts, derived from configuration and
/// holding no sensitive values: the capabilities record THAT a database URL or
/// an API base was supplied, never what it was.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartupPlan {
    pub bind: String,
    pub capabilities: Vec<BackendCapability>,
}

pub fn startup_plan(config: &WebConfig) -> Result<StartupPlan, WebError> {
    let bind = non_empty("GHA_INDIE_WORKER_WEB_BIND", &config.bind)?;
    let optional_capabilities = [
        config.database_url.as_ref().map(|value| {
            (
                BackendCapability::DirectReadOnlyDatabase,
                "GHA_INDIE_WORKER_DATABASE_URL",
                value,
            )
        }),
        config.api_http_base.as_ref().map(|value| {
            (
                BackendCapability::StatelessHttp,
                "GHA_INDIE_WORKER_API_HTTP_BASE",
                value,
            )
        }),
    ];
    let capabilities = optional_capabilities
        .into_iter()
        .flatten()
        .map(|(capability, field, value)| non_empty(field, value).map(|_| capability))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(StartupPlan { bind, capabilities })
}

fn non_empty(field: &'static str, value: &str) -> Result<String, WebError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(WebError::InvalidConfiguration(field));
    }
    Ok(value.to_owned())
}

pub fn router() -> Router {
    Router::new()
        .route("/", get(|| async { Html(pages::home::markup()) }))
        .route("/health", get(|| async { StatusCode::NO_CONTENT }))
        .route("/healthz", get(|| async { StatusCode::NO_CONTENT }))
        .route("/readyz", get(|| async { StatusCode::NO_CONTENT }))
}

pub async fn run(config: &WebConfig) -> Result<(), Box<dyn Error>> {
    // Fail closed on blank configuration before opening a socket.
    let plan = startup_plan(config)?;
    let listener = tokio::net::TcpListener::bind(&plan.bind).await?;
    eprintln!(
        "gha-indie-worker-web-server listening on {} with capabilities {:?}",
        listener.local_addr()?,
        plan.capabilities
    );
    axum::serve(listener, router()).await?;
    Ok(())
}

#[cfg(test)]
mod startup_plan_tests {
    use super::{startup_plan, BackendCapability, StartupPlan};
    use crate::{config::WebConfig, error::WebError};

    #[test]
    fn startup_plan_derives_capabilities_without_retaining_the_database_url() {
        let config = WebConfig {
            bind: " 127.0.0.1:8081 ".into(),
            api_http_base: Some("http://api:8080".into()),
            database_url: Some("postgres://sensitive-value".into()),
        };

        let plan = startup_plan(&config).expect("valid web startup plan");

        assert_eq!(
            plan,
            StartupPlan {
                bind: "127.0.0.1:8081".into(),
                capabilities: vec![
                    BackendCapability::DirectReadOnlyDatabase,
                    BackendCapability::StatelessHttp,
                ],
            }
        );
        assert!(!format!("{plan:?}").contains("sensitive-value"));
    }

    #[test]
    fn startup_plan_rejects_blank_optional_configuration() {
        let error = startup_plan(&WebConfig {
            bind: "127.0.0.1:8081".into(),
            api_http_base: Some("  ".into()),
            database_url: None,
        })
        .expect_err("blank API base must fail closed");

        assert!(matches!(
            error,
            WebError::InvalidConfiguration("GHA_INDIE_WORKER_API_HTTP_BASE")
        ));
    }
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
        for path in ["/health", "/healthz", "/readyz"] {
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
