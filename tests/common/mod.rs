// Not every test binary uses every helper; each one compiles this module
// separately, so unused items here are expected rather than a smell.
#![allow(dead_code)]

//! Shared harness for the HTTP surface tests.
//!
//! Everything here is offline: [`state`] builds an `AppState` with no database
//! pool, no shared-auth client and an api-server base that is never called, and
//! `send` stops short of ores-middleware so no test depends on the ambient
//! environment.

use axum::body::Body;
use axum::http::{Request, Response, StatusCode};
use gha_indie_worker_web_server::config::WebConfig;
use gha_indie_worker_web_server::server;
use gha_indie_worker_web_server::state::AppState;
use http_body_util::BodyExt;
use tower::ServiceExt;

pub const APP_HOST: &str = "app.indiebuild.dev";
pub const USER_HOST: &str = "user.indiebuild.dev";
pub const ORG_HOST: &str = "org.indiebuild.dev";
pub const MOBILE_HOST: &str = "m.indiebuild.dev";

#[must_use]
pub fn state() -> AppState {
    AppState::for_tests(WebConfig::for_tests())
}

/// One request against the whole HTTP surface.
pub async fn send(state: &AppState, request: Request<Body>) -> Response<Body> {
    server::build_router(state)
        .oneshot(request)
        .await
        .expect("the router is infallible")
}

/// `GET path` with an explicit `Host`.
pub async fn get(state: &AppState, host: &str, path: &str) -> Response<Body> {
    let request = Request::builder()
        .method("GET")
        .uri(path)
        .header("host", host)
        .body(Body::empty())
        .expect("a valid request");
    send(state, request).await
}

/// `POST path` with a form body and an optional `X-CSRF-Token` header.
pub async fn post_form(
    state: &AppState,
    host: &str,
    path: &str,
    body: &str,
    csrf_header: Option<&str>,
) -> Response<Body> {
    let mut builder = Request::builder()
        .method("POST")
        .uri(path)
        .header("host", host)
        .header("content-type", "application/x-www-form-urlencoded");
    if let Some(token) = csrf_header {
        builder = builder.header("x-csrf-token", token);
    }
    send(
        state,
        builder.body(Body::from(body.to_owned())).expect("a valid request"),
    )
    .await
}

/// The response body as text.
pub async fn text(response: Response<Body>) -> String {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("a complete body")
        .to_bytes();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Status plus body, which is what most assertions want.
pub async fn status_and_text(response: Response<Body>) -> (StatusCode, String) {
    let status = response.status();
    (status, text(response).await)
}
