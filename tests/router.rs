//! Router-level tests: drive the application with `oneshot`, never bind a socket.
//!
//! These exercise the seams the unit tests cannot reach on their own — that host routing, the
//! security headers, the CSRF refusal and the asset registry all actually meet in a response.
//!
//! The router under test is [`gha_indie_worker_web_server::pages::router`], deliberately *without*
//! the ORES middleware stack: this file is about what this crate decides, and wrapping it would
//! mean these assertions passed or failed on the middleware's environment instead.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use gha_indie_worker_web_server::{config::WebConfig, pages, state::AppState};
use http_body_util::BodyExt;
use tower::ServiceExt;

fn app() -> axum::Router {
    let mut config = WebConfig::from_env();
    config.apex = "indiebuild.dev".to_owned();
    config.public_url = "https://app.indiebuild.dev".to_owned();
    config.bind = "127.0.0.1:0".to_owned();
    pages::router(AppState::new(config))
}

async fn text(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn get(host: &str, path: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(path)
        .header(header::HOST, host)
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn the_admin_hosts_are_not_served_by_the_product_binary() {
    for host in [
        "admin.indiebuild.dev",
        "admin-api.indiebuild.dev",
        "api-admin.indiebuild.dev",
        "api.indiebuild.dev",
        "auth.indiebuild.dev",
        "app.indiebuild.dev.evil.test",
    ] {
        let response = app().oneshot(get(host, "/")).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "host {host}");
    }
}

#[tokio::test]
async fn the_landing_page_offers_two_equal_doors() {
    let response = app().oneshot(get("indiebuild.dev", "/")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = text(response).await;
    assert!(body.contains("For your team"), "{body}");
    assert!(body.contains("For yourself"), "{body}");
    assert!(body.contains("https://org.indiebuild.dev/new"));
    assert!(body.contains("https://user.indiebuild.dev/signup"));
    assert_eq!(body.matches("class=\"door\"").count(), 2);
}

#[tokio::test]
async fn every_response_carries_the_hardening_headers() {
    let response = app()
        .oneshot(get("user.indiebuild.dev", "/"))
        .await
        .unwrap();
    let headers = response.headers().clone();
    let policy = headers
        .get(header::CONTENT_SECURITY_POLICY)
        .and_then(|value| value.to_str().ok())
        .expect("a content security policy");
    assert!(policy.contains("default-src 'none'"), "{policy}");
    assert!(!policy.contains("unsafe-inline"), "{policy}");
    assert!(!policy.contains("unsafe-eval"), "{policy}");
    assert_eq!(headers.get(header::X_FRAME_OPTIONS).unwrap(), "DENY");
    assert_eq!(
        headers.get(header::X_CONTENT_TYPE_OPTIONS).unwrap(),
        "nosniff"
    );
    assert!(headers.contains_key(header::REFERRER_POLICY));
    assert!(headers.contains_key("permissions-policy"));
    assert!(headers.contains_key(header::STRICT_TRANSPORT_SECURITY));
    assert_eq!(headers.get(header::CACHE_CONTROL).unwrap(), "no-store");
    // And a fresh visitor is issued the CSRF cookie the forms will double-submit.
    let cookie = headers
        .get(header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .expect("a csrf cookie");
    assert!(cookie.starts_with("giw_csrf="));
    for attribute in ["HttpOnly", "Secure", "SameSite=Lax", "Path=/"] {
        assert!(
            cookie.contains(attribute),
            "{attribute} missing from {cookie}"
        );
    }
}

#[tokio::test]
async fn an_anonymous_visitor_to_the_app_shell_is_sent_to_a_login_surface() {
    let response = app()
        .oneshot(get("app.indiebuild.dev", "/runs"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers().get(header::LOCATION).unwrap(),
        "https://user.indiebuild.dev/"
    );

    // An org visitor is never bounced to the individual login.
    let response = app().oneshot(get("org.indiebuild.dev", "/")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(text(response)
        .await
        .contains("Sign in to your organization"));
}

#[tokio::test]
async fn assets_are_served_from_our_own_origin_with_their_digest() {
    let response = app()
        .oneshot(get("app.indiebuild.dev", "/assets/app.css"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "text/css; charset=utf-8"
    );
    let etag = response
        .headers()
        .get(header::ETAG)
        .and_then(|value| value.to_str().ok())
        .expect("an etag");
    assert!(etag.contains("sha384-"), "{etag}");

    let missing = app()
        .oneshot(get("app.indiebuild.dev", "/assets/nope.js"))
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_state_changing_post_without_a_token_is_refused() {
    let request = Request::builder()
        .method("POST")
        .uri("/org/invitations")
        .header(header::HOST, "org.indiebuild.dev")
        .header(header::ORIGIN, "https://org.indiebuild.dev")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from("email=jo%40acme.test&role=member"))
        .unwrap();
    let response = app().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_cross_origin_post_is_refused_even_with_a_matching_double_submit() {
    let token = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let request = Request::builder()
        .method("POST")
        .uri("/org/invitations")
        .header(header::HOST, "org.indiebuild.dev")
        .header(header::ORIGIN, "https://evil.test")
        .header(header::COOKIE, format!("giw_csrf={token}"))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(format!(
            "csrf_token={token}&email=jo%40acme.test"
        )))
        .unwrap();
    let response = app().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn the_probes_answer_without_a_host_of_ours() {
    let health = app()
        .oneshot(get("10.0.0.1:8080", "/healthz"))
        .await
        .unwrap();
    assert_eq!(health.status(), StatusCode::OK);
    assert!(text(health).await.contains("gha-indie-worker-web-server"));

    let ready = app()
        .oneshot(get("10.0.0.1:8080", "/readyz"))
        .await
        .unwrap();
    assert_eq!(ready.status(), StatusCode::OK);
}

#[tokio::test]
async fn an_unknown_path_is_a_404_that_names_the_surface_it_reached() {
    let response = app()
        .oneshot(get("org.indiebuild.dev", "/nothing-here"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = text(response).await;
    assert!(body.contains("org"), "{body}");
}
