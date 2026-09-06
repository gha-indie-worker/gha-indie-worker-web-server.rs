//! Host routing across the whole HTTP surface.
//!
//! These are the tests that pin the product's front door: one process, four
//! hosts, and an unknown host that gets nothing.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{get, send, state, status_and_text, text, APP_HOST, MOBILE_HOST, ORG_HOST, USER_HOST};

#[tokio::test]
async fn the_app_home_offers_both_entry_points() {
    let (status, body) = status_and_text(get(&state(), APP_HOST, "/").await).await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("https://org.indiebuild.dev/login"),
        "missing the organization entry point"
    );
    assert!(
        body.contains("https://user.indiebuild.dev/login"),
        "missing the personal entry point"
    );
    assert!(body.contains("Sign in to your organization"));
    assert!(body.contains("Personal account"));
}

#[tokio::test]
async fn the_app_home_is_continuous_with_the_marketing_site() {
    let body = text(get(&state(), APP_HOST, "/").await).await;

    // Same mark, same accent, same numbered-eyebrow rhythm as the Astro site.
    assert!(body.contains("GHA/IW"));
    assert!(body.contains("#67c2a3"));
    assert!(body.contains("01 / PRINCIPLES"));
    assert!(body.contains("03 / ACCESS"));
}

#[tokio::test]
async fn the_org_login_page_is_served_on_the_org_host() {
    let (status, body) = status_and_text(get(&state(), ORG_HOST, "/login").await).await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Sign in to your organization"));
    assert!(body.contains(r#"data-surface="org""#));
}

#[tokio::test]
async fn an_unknown_host_gets_a_404_page_and_no_product_chrome() {
    let (status, body) = status_and_text(get(&state(), "evil.example", "/").await).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body.contains("Nothing here"));
    assert!(!body.contains("https://org.indiebuild.dev/login"));
    assert!(!body.contains(r#"class="primary-nav""#));
}

#[tokio::test]
async fn an_unknown_host_gets_404_on_every_path_including_health() {
    let state = state();
    for path in ["/", "/login", "/runs", "/healthz", "/onboarding/create"] {
        let response = get(&state, "not-ours.example", path).await;
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "{path} leaked on an unknown host"
        );
    }
}

#[tokio::test]
async fn a_missing_host_header_is_not_a_product_surface() {
    let request = Request::builder()
        .method("GET")
        .uri("/")
        .body(Body::empty())
        .expect("a valid request");
    let response = send(&state(), request).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn each_surface_reports_itself_on_the_body_element() {
    let state = state();
    for (host, label) in [
        (APP_HOST, "app"),
        (USER_HOST, "user"),
        (ORG_HOST, "org"),
        (MOBILE_HOST, "m"),
    ] {
        // user. and org. are login-gated: a signed-out visitor at their root is redirected
        // to /login (see the redirect test above), so ask them for the page they serve.
        let path = if host == USER_HOST || host == ORG_HOST {
            "/login"
        } else {
            "/"
        };
        let body = text(get(&state, host, path).await).await;
        assert!(
            body.contains(&format!(r#"data-surface="{label}""#)),
            "{host} did not report itself"
        );
    }
}

#[tokio::test]
async fn org_and_user_each_own_their_own_login() {
    let state = state();

    let org = text(get(&state, ORG_HOST, "/login").await).await;
    assert!(org.contains("ORGANIZATIONS"));
    assert!(org.contains(r#"hx-post="/login""#));

    let user = text(get(&state, USER_HOST, "/login").await).await;
    assert!(user.contains("INDIVIDUALS"));
    assert!(user.contains(r#"hx-post="/login""#));

    // Same path, different surface, different page.
    assert_ne!(org, user);
}

#[tokio::test]
async fn a_forwarded_host_from_an_untrusted_peer_is_ignored() {
    // `oneshot` carries no ConnectInfo, so no peer can be trusted and the
    // `Host` header must win outright.
    let request = Request::builder()
        .method("GET")
        .uri("/")
        .header("host", APP_HOST)
        .header("x-forwarded-host", ORG_HOST)
        .body(Body::empty())
        .expect("a valid request");
    let body = text(send(&state(), request).await).await;

    assert!(
        body.contains(r#"data-surface="app""#),
        "an untrusted proxy header changed the surface"
    );
}

#[tokio::test]
async fn the_host_header_is_case_and_port_insensitive() {
    let (status, body) = status_and_text(get(&state(), "APP.IndieBuild.dev:443", "/").await).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains(r#"data-surface="app""#));
}

#[tokio::test]
async fn health_is_served_on_every_product_surface() {
    let state = state();
    for host in [APP_HOST, USER_HOST, ORG_HOST, MOBILE_HOST] {
        let (status, body) = status_and_text(get(&state, host, "/healthz").await).await;
        assert_eq!(status, StatusCode::OK, "{host} is not reporting health");
        assert!(body.contains("\"status\":\"ok\""));
    }
}

#[tokio::test]
async fn every_page_carries_the_security_headers_and_a_nonced_policy() {
    let response = get(&state(), APP_HOST, "/").await;
    let headers = response.headers().clone();

    let policy = headers
        .get("content-security-policy")
        .and_then(|value| value.to_str().ok())
        .expect("a policy")
        .to_owned();
    assert!(policy.contains("script-src 'self' 'nonce-"));
    assert!(!policy.contains("unsafe-inline"));
    assert_eq!(
        headers.get("x-content-type-options").and_then(|v| v.to_str().ok()),
        Some("nosniff")
    );
    assert_eq!(
        headers.get("x-frame-options").and_then(|v| v.to_str().ok()),
        Some("DENY")
    );
    assert_eq!(
        headers.get("cache-control").and_then(|v| v.to_str().ok()),
        Some("private, no-store")
    );

    // The nonce in the policy is the nonce on the page's script and style.
    let nonce = policy
        .split("'nonce-")
        .nth(1)
        .and_then(|rest| rest.split('\'').next())
        .expect("a nonce")
        .to_owned();
    let body = text(response).await;
    assert!(body.contains(&format!(r#"nonce="{nonce}""#)));
}

#[tokio::test]
async fn nonces_are_not_reused_between_requests() {
    let state = state();
    let first = get(&state, APP_HOST, "/").await;
    let second = get(&state, APP_HOST, "/").await;

    let policy_of = |response: &axum::http::Response<Body>| {
        response
            .headers()
            .get("content-security-policy")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
            .expect("a policy")
    };
    assert_ne!(policy_of(&first), policy_of(&second));
}

#[tokio::test]
async fn a_signed_out_visitor_is_redirected_to_login_rather_than_shown_the_product() {
    let response = get(&state(), APP_HOST, "/runs").await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let location = response
        .headers()
        .get("location")
        .and_then(|value| value.to_str().ok())
        .expect("a redirect target");
    assert!(location.starts_with("https://user.indiebuild.dev/login"));
    assert!(
        location.contains("next=/runs"),
        "the redirect must remember the page: {location}"
    );
}
