#![forbid(unsafe_code)]

mod common;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{get, send, state, text, APP_HOST, MOBILE_HOST, ORG_HOST, USER_HOST};
use gha_indie_worker_web_server::csrf::{ANONYMOUS_SUBJECT, CSRF_HEADER};

const RELEASE_PATHS: [&str; 2] = ["/releases", "/releases.json"];
const MUTATING_METHODS: [&str; 4] = ["POST", "PUT", "PATCH", "DELETE"];

#[tokio::test]
async fn release_routes_require_a_session_and_do_not_cross_surfaces() {
    let app = state();
    for path in RELEASE_PATHS {
        for host in [APP_HOST, MOBILE_HOST] {
            let response = get(&app, host, path).await;
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{host}{path}");
            assert_eq!(response.headers()["cache-control"], "private, no-store");
            assert_eq!(response.headers()["vary"], "Cookie, Authorization, Host");
            assert_eq!(response.headers()["x-content-type-options"], "nosniff");
            assert_eq!(response.headers()["x-frame-options"], "DENY");
        }
        for host in ["evil.example", "admin.indiebuild.dev", ORG_HOST, USER_HOST] {
            assert_eq!(get(&app, host, path).await.status(), StatusCode::NOT_FOUND);
        }
    }
}

#[tokio::test]
async fn release_dashboard_has_no_mutating_method() {
    let app = state();
    for path in RELEASE_PATHS {
        for host in [APP_HOST, MOBILE_HOST] {
            for method in MUTATING_METHODS {
                let request = Request::builder()
                    .method(method)
                    .uri(path)
                    .header("host", host)
                    .body(Body::empty())
                    .unwrap();
                let response = send(&app, request).await;
                assert_eq!(response.status(), StatusCode::FORBIDDEN, "{method} {host}{path}");
                assert_eq!(response.headers()["cache-control"], "private, no-store");
            }
        }
    }
}

#[tokio::test]
async fn valid_csrf_cannot_turn_a_release_route_into_a_write_route() {
    let app = state();
    let token = app.csrf.issue(ANONYMOUS_SUBJECT);
    for path in RELEASE_PATHS {
        for host in [APP_HOST, MOBILE_HOST] {
            for method in MUTATING_METHODS {
                let request = Request::builder()
                    .method(method)
                    .uri(path)
                    .header("host", host)
                    .header(CSRF_HEADER, token.as_str())
                    .body(Body::empty())
                    .unwrap();
                let response = send(&app, request).await;
                // a csrf rejection would not prove that method dispatch remains read-only
                assert_eq!(
                    response.status(),
                    StatusCode::METHOD_NOT_ALLOWED,
                    "{method} {host}{path}"
                );
            }
        }
    }
}

#[tokio::test]
async fn head_requests_do_not_bypass_release_authentication() {
    let app = state();
    for path in RELEASE_PATHS {
        for host in [APP_HOST, MOBILE_HOST] {
            let request = Request::builder()
                .method("HEAD")
                .uri(path)
                .header("host", host)
                .body(Body::empty())
                .unwrap();
            let response = send(&app, request).await;
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            assert_eq!(response.headers()["cache-control"], "private, no-store");
            assert!(text(response).await.is_empty());
        }
    }
}

#[tokio::test]
async fn malformed_or_authority_shaped_query_fields_are_rejected() {
    let app = state();
    for path in RELEASE_PATHS {
        for host in [APP_HOST, MOBILE_HOST] {
            for query in [
                "org_id=another-tenant",
                "promotionAllowed=true",
                "limit=not-a-number",
                "limit=-1",
                "limit=18446744073709551616",
                "limit=1&limit=2",
                "repository=a%2Fb&repository=c%2Fd",
            ] {
                let response = get(&app, host, &format!("{path}?{query}")).await;
                assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{host}{path}?{query}");
                assert_eq!(response.headers()["cache-control"], "private, no-store");
            }
        }
    }
}

#[tokio::test]
async fn an_untrusted_forwarded_host_cannot_select_the_release_surface() {
    let app = state();
    for path in RELEASE_PATHS {
        for host in ["evil.example", ORG_HOST, USER_HOST] {
            let request = Request::builder()
                .uri(path)
                .header("host", host)
                .header("x-forwarded-host", APP_HOST)
                .body(Body::empty())
                .unwrap();
            assert_eq!(send(&app, request).await.status(), StatusCode::NOT_FOUND);
        }
    }
}

#[tokio::test]
async fn missing_host_and_htmx_headers_do_not_grant_release_access() {
    let app = state();
    for path in RELEASE_PATHS {
        let request = Request::builder()
            .uri(path)
            .header("x-forwarded-host", APP_HOST)
            .header("hx-request", "true")
            .body(Body::empty())
            .unwrap();
        assert_eq!(send(&app, request).await.status(), StatusCode::NOT_FOUND);
        for host in [APP_HOST, MOBILE_HOST] {
            let request = Request::builder()
                .uri(path)
                .header("host", host)
                .header("hx-request", "true")
                .body(Body::empty())
                .unwrap();
            let response = send(&app, request).await;
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            assert_eq!(response.headers()["cache-control"], "private, no-store");
        }
    }
}
