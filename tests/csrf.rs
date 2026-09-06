//! CSRF, end to end.
//!
//! The middleware rejects an unsafe request before it reaches a handler, and the
//! handler re-checks the form field for the no-JavaScript path. Both are
//! exercised here against the real router.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{get, post_form, send, state, text, APP_HOST, ORG_HOST};
use gha_indie_worker_web_server::csrf::ANONYMOUS_SUBJECT;

#[tokio::test]
async fn a_post_with_no_token_at_all_is_refused() {
    let request = Request::builder()
        .method("POST")
        .uri("/logout")
        .header("host", APP_HOST)
        .body(Body::empty())
        .expect("a valid request");
    let response = send(&state(), request).await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_form_post_with_no_token_is_refused_by_the_handler() {
    let response = post_form(&state(), APP_HOST, "/logout", "", None).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(text(response).await.contains("That form expired"));
}

#[tokio::test]
async fn a_forged_header_token_is_refused() {
    let response = post_form(&state(), APP_HOST, "/logout", "", Some("deadbeef.deadbeef")).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_token_minted_for_another_subject_is_refused() {
    let state = state();
    let other_subject = state.csrf.issue("someone-else");
    let response = post_form(&state, APP_HOST, "/logout", "", Some(&other_subject)).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_valid_header_token_is_accepted() {
    let state = state();
    let token = state.csrf.issue(ANONYMOUS_SUBJECT);
    let response = post_form(&state, APP_HOST, "/logout", "", Some(&token)).await;

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers().get("location").and_then(|value| value.to_str().ok()),
        Some("/")
    );
}

#[tokio::test]
async fn a_valid_form_field_is_accepted_without_a_header() {
    let state = state();
    let token = state.csrf.issue(ANONYMOUS_SUBJECT);
    let body = format!("csrf_token={token}");
    let response = post_form(&state, APP_HOST, "/logout", &body, None).await;

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn logging_out_clears_the_host_scoped_cookie() {
    let state = state();
    let token = state.csrf.issue(ANONYMOUS_SUBJECT);
    let response = post_form(&state, APP_HOST, "/logout", "", Some(&token)).await;

    let cookie = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| value.starts_with("giw_session="))
        .expect("the session cookie is cleared")
        .to_owned();

    assert!(cookie.contains("Max-Age=0"));
    assert!(cookie.contains("HttpOnly"));
    assert!(
        !cookie.to_ascii_lowercase().contains("domain="),
        "the session cookie must stay host-scoped"
    );
}

#[tokio::test]
async fn a_get_never_needs_a_token() {
    let state = state();
    for (host, path) in [(APP_HOST, "/"), (ORG_HOST, "/login"), (ORG_HOST, "/onboarding/create")] {
        let response = get(&state, host, path).await;
        assert_eq!(response.status(), StatusCode::OK, "{host}{path}");
    }
}

#[tokio::test]
async fn a_page_hands_the_browser_a_usable_token() {
    let state = state();
    let response = get(&state, APP_HOST, "/").await;

    let cookie = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| value.starts_with("giw_csrf="))
        .expect("a csrf cookie")
        .to_owned();
    // htmx has to read it, so it is deliberately not HttpOnly.
    assert!(!cookie.contains("HttpOnly"));

    let token = cookie
        .trim_start_matches("giw_csrf=")
        .split(';')
        .next()
        .expect("a value")
        .to_owned();

    // The same token is on the page, and it works.
    let body = text(response).await;
    assert!(body.contains(&token), "the page must carry the token it set");

    let accepted = post_form(&state, APP_HOST, "/logout", "", Some(&token)).await;
    assert_eq!(accepted.status(), StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn the_onboarding_wizard_refuses_an_unauthenticated_forgery() {
    let response = post_form(&state(), ORG_HOST, "/onboarding/create", "name=Acme", None).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn an_unknown_wizard_step_is_a_404_not_a_half_built_organization() {
    let state = state();
    let token = state.csrf.issue(ANONYMOUS_SUBJECT);
    let response = post_form(&state, ORG_HOST, "/onboarding/not-a-step", "name=Acme", Some(&token)).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
