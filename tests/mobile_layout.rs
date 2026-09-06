//! Layout selection for `m.indiebuild.dev`.
//!
//! Mobile is the same product served through a compact shell. These tests pin
//! that the shell actually changes — and that nothing on it depends on hover.

mod common;

use axum::http::StatusCode;
use common::{get, state, status_and_text, text, APP_HOST, MOBILE_HOST};
use gha_indie_worker_web_server::ui::styles::CSS;

#[tokio::test]
async fn the_mobile_host_selects_the_compact_shell() {
    let (status, body) = status_and_text(get(&state(), MOBILE_HOST, "/").await).await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.contains(r#"<body class="compact""#));
    assert!(body.contains(r#"data-surface="m""#));
}

#[tokio::test]
async fn the_desktop_host_does_not() {
    let body = text(get(&state(), APP_HOST, "/").await).await;

    assert!(body.contains(r#"<body class="wide""#));
    assert!(!body.contains(r#"<body class="compact""#));
    assert!(!body.contains(r#"class="bottom-nav""#));
}

#[tokio::test]
async fn mobile_serves_the_same_pages_as_the_app_surface() {
    let state = state();
    // Pages that need no upstream: the harness has no api-server behind it.
    for path in ["/", "/healthz"] {
        let response = get(&state, MOBILE_HOST, path).await;
        assert_eq!(response.status(), StatusCode::OK, "{path} is missing on mobile");
    }
    // The product routes exist on both, so a signed-out visit redirects the
    // same way rather than 404ing.
    let runs = get(&state, MOBILE_HOST, "/runs").await;
    assert_eq!(runs.status(), StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn mobile_reaches_the_personal_pages_under_a_prefix() {
    let response = get(&state(), MOBILE_HOST, "/me/login").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(text(response).await.contains("INDIVIDUALS"));
}

#[tokio::test]
async fn the_two_surfaces_render_the_same_content_through_different_shells() {
    let state = state();
    let mobile = text(get(&state, MOBILE_HOST, "/").await).await;
    let desktop = text(get(&state, APP_HOST, "/").await).await;

    // Same content…
    for marker in ["01 / PRINCIPLES", "03 / ACCESS", "https://org.indiebuild.dev/login"] {
        assert!(mobile.contains(marker), "mobile is missing {marker}");
        assert!(desktop.contains(marker), "desktop is missing {marker}");
    }
    // …different shell.
    assert_ne!(mobile, desktop);
}

#[test]
fn the_compact_layout_never_depends_on_hover() {
    // Every `:hover` rule in the stylesheet is paired with a `:focus-visible`
    // rule, so a keyboard or touch user reaches the same state.
    let hover_rules = CSS.matches(":hover").count();
    let focus_rules = CSS.matches(":focus-visible").count();
    assert!(
        focus_rules >= hover_rules,
        "{hover_rules} hover rules but only {focus_rules} focus rules — a control may be hover-only"
    );

    // And no rule uses hover to *reveal* anything.
    for line in CSS.lines().filter(|line| line.contains(":hover")) {
        assert!(!line.contains("display:"), "hover must not control visibility: {line}");
        assert!(
            !line.contains("visibility:"),
            "hover must not control visibility: {line}"
        );
        assert!(
            !line.contains("opacity: 1"),
            "hover must not control visibility: {line}"
        );
    }
}

#[test]
fn the_bottom_bar_is_reachable_and_respects_the_safe_area() {
    assert!(
        CSS.contains("min-height: 3rem"),
        "touch targets must be at least 3rem tall"
    );
    assert!(
        CSS.contains("env(safe-area-inset-bottom)"),
        "the bar must clear the home indicator"
    );
    assert!(
        CSS.contains("body.compact .bottom-nav { display: grid"),
        "the bar is compact-only"
    );
    assert!(
        CSS.contains("body.compact .primary-nav { display: none"),
        "compact hides the inline nav"
    );
}

#[tokio::test]
async fn the_bottom_bar_marks_the_current_destination() {
    // Signed out on `m.`, the marketing destinations are what the bar offers.
    let body = text(get(&state(), MOBILE_HOST, "/").await).await;
    assert!(body.contains(r#"class="bottom-nav""#));
    assert_eq!(
        body.matches(r#"<nav class="bottom-nav" aria-label="Primary">"#).count(),
        1
    );
    assert!(
        body.contains(r#"aria-hidden="true""#),
        "glyphs must be hidden from assistive tech"
    );
}
