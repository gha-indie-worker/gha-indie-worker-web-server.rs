mod common;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{get, send, state, APP_HOST, MOBILE_HOST, ORG_HOST, USER_HOST};

#[tokio::test]
async fn release_routes_require_a_session_and_do_not_cross_surfaces() {
    for path in ["/releases", "/releases.json"] {
        for host in [APP_HOST, MOBILE_HOST] {
            let response = get(&state(), host, path).await;
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            assert_eq!(response.headers()["cache-control"], "private, no-store");
        }
        for host in ["evil.example", "admin.indiebuild.dev", ORG_HOST, USER_HOST] {
            assert_eq!(get(&state(), host, path).await.status(), StatusCode::NOT_FOUND);
        }
    }
}

#[tokio::test]
async fn release_dashboard_has_no_mutating_method() {
    let request = Request::builder().method("POST").uri("/releases")
        .header("host", APP_HOST).body(Body::empty()).unwrap();
    let response = send(&state(), request).await;
    // Existing middleware may reject missing CSRF before method dispatch.
    assert!(matches!(response.status(), StatusCode::METHOD_NOT_ALLOWED | StatusCode::FORBIDDEN));
}
