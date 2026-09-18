#![forbid(unsafe_code)]

use crate::{auth, config::WebConfig, error::WebError, pages};
use axum::{
    extract::Path,
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
    Router,
};
use std::error::Error;

fn verified_bearer(headers: &HeaderMap) -> bool {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| auth::require_bearer(Some(value)).ok())
        .is_some()
}

async fn app_root(headers: HeaderMap) -> Response {
    if verified_bearer(&headers) {
        Redirect::to("/app/overview").into_response()
    } else {
        Redirect::to("/login").into_response()
    }
}

async fn app_section(headers: HeaderMap, Path(section): Path<String>) -> Response {
    if !verified_bearer(&headers) {
        return Redirect::to("/login").into_response();
    }
    Html(pages::dashboard::markup(&section)).into_response()
}

async fn auth_start() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Html(
            "<!doctype html><html><body><h1>Identity exchange not configured</h1><p>This branch fails closed until shared-auth is connected. No development identity is minted.</p><a href=\"/login\">Back to sign in</a></body></html>"
                .to_string(),
        ),
    )
        .into_response()
}

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
        .route("/login", get(|| async { Html(pages::login::markup()) }))
        .route("/auth/start", get(auth_start))
        .route("/app", get(app_root))
        .route("/app/{section}", get(app_section))
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
mod tests {
    use super::*;
    use axum::{
        body::{to_bytes, Body},
        http::{Method, Request},
    };
    use tower::ServiceExt;

    async fn request(
        method: Method,
        path: &str,
        bearer: Option<&str>,
    ) -> (StatusCode, HeaderMap, String) {
        let mut builder = Request::builder().method(method).uri(path);
        if let Some(token) = bearer {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let response = router()
            .oneshot(builder.body(Body::empty()).expect("request"))
            .await
            .expect("router response");
        let status = response.status();
        let headers = response.headers().clone();
        let body = to_bytes(response.into_body(), 256 * 1024)
            .await
            .expect("response body");
        (
            status,
            headers,
            String::from_utf8(body.to_vec()).expect("utf8 body"),
        )
    }

    #[tokio::test]
    async fn home_surface_is_html_and_has_login_entry() {
        let (status, headers, body) = request(Method::GET, "/", None).await;
        assert_eq!(status, StatusCode::OK);
        assert!(headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .starts_with("text/html"));
        assert!(body.contains("IndieBuild"));
        assert!(body.contains("href=\"/login\""));
    }

    #[tokio::test]
    async fn signed_out_dashboard_redirects_to_login() {
        let (status, headers, _) = request(Method::GET, "/app/pipelines", None).await;
        assert!(status.is_redirection());
        assert_eq!(
            headers.get(header::LOCATION).and_then(|v| v.to_str().ok()),
            Some("/login")
        );
    }

    #[tokio::test]
    async fn bearer_identity_reaches_operational_dashboard() {
        let (status, _, body) = request(Method::GET, "/app/pipelines", Some("test-token")).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("Run attempt DAG"));
        assert!(body.contains("Infrastructure"));
        assert!(body.contains("Errors"));
        assert!(body.contains("Ecosystem"));
    }

    #[tokio::test]
    async fn login_is_real_surface_but_identity_start_fails_closed_until_wired() {
        let (login_status, _, login_body) = request(Method::GET, "/login", None).await;
        assert_eq!(login_status, StatusCode::OK);
        assert!(login_body.contains("Continue with IndieBuild"));

        let (start_status, _, start_body) = request(Method::GET, "/auth/start", None).await;
        assert_eq!(start_status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(start_body.contains("shared-auth"));
        assert!(start_body.contains("fails closed"));
    }

    #[tokio::test]
    async fn app_root_preserves_auth_boundary() {
        let (anonymous, anonymous_headers, _) = request(Method::GET, "/app", None).await;
        assert!(anonymous.is_redirection());
        assert_eq!(
            anonymous_headers
                .get(header::LOCATION)
                .and_then(|v| v.to_str().ok()),
            Some("/login")
        );

        let (authenticated, authenticated_headers, _) =
            request(Method::GET, "/app", Some("test-token")).await;
        assert!(authenticated.is_redirection());
        assert_eq!(
            authenticated_headers
                .get(header::LOCATION)
                .and_then(|v| v.to_str().ok()),
            Some("/app/overview")
        );
    }

    #[tokio::test]
    async fn health_and_ready_are_empty_success_responses() {
        for path in ["/health", "/healthz", "/readyz"] {
            let (status, _, body) = request(Method::GET, path, None).await;
            assert_eq!(status, StatusCode::NO_CONTENT);
            assert!(body.is_empty());
        }
    }

    #[tokio::test]
    async fn router_fails_closed_for_unknown_routes_and_wrong_methods() {
        let (missing, _, _) = request(Method::GET, "/does-not-exist", None).await;
        let (wrong_method, _, _) = request(Method::POST, "/", None).await;
        assert_eq!(missing, StatusCode::NOT_FOUND);
        assert_eq!(wrong_method, StatusCode::METHOD_NOT_ALLOWED);
    }
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
