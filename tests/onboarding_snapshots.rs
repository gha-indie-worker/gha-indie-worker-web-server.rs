//! Rendering snapshots for the organization onboarding wizard.
//!
//! Every step is rendered twice: once directly from [`org::step_body`], which
//! pins the markup, and once through the real HTTP surface, which pins that the
//! step is actually reachable at its own URL on `org.indiebuild.dev`.
//!
//! These are assertions about *contract*, not about pixels: each step must name
//! itself, show the rail with exactly one current entry, and post back to its own
//! path with a CSRF token.

mod common;

use axum::http::StatusCode;
use common::{get, state, status_and_text, ORG_HOST};
use gha_indie_worker_web_server::csrf::CsrfCheck;
use gha_indie_worker_web_server::hosts::org::{self, OnboardingStep, WizardView};
use gha_indie_worker_web_server::hosts::Surface;
use gha_indie_worker_web_server::middleware::RequestCtx;

fn context() -> RequestCtx {
    RequestCtx {
        surface: Surface::Org,
        host: "org.indiebuild.dev".into(),
        base_domain: "indiebuild.dev".into(),
        path: "/onboarding".into(),
        nonce: "n0nce".into(),
        csrf_token: "tok3n".into(),
        csrf: CsrfCheck::NotRequired,
        actor: None,
        is_htmx: false,
        subject: "anonymous".into(),
        release_manifest_url: None,
        chat_enabled: true,
    }
}

fn render(step: OnboardingStep, view: &WizardView) -> String {
    org::step_body(&context(), step, view).into_string()
}

#[test]
fn step_one_asks_for_a_name_and_an_optional_slug() {
    let html = render(OnboardingStep::CreateOrg, &WizardView::default());

    assert!(html.contains("01 / ONBOARDING"));
    assert!(html.contains("<h1>Create organization</h1>"));
    assert!(html.contains(r#"<label for="org-name">Organization name</label>"#));
    assert!(html.contains(r#"<label for="org-slug">URL slug</label>"#));
    assert!(html.contains(r#"hx-post="/onboarding/create""#));
    assert!(html.contains(r#"name="csrf_token" value="tok3n""#));
}

#[test]
fn step_two_offers_dns_and_email_proof() {
    let html = render(OnboardingStep::VerifyDomain, &WizardView::default());

    assert!(html.contains("02 / ONBOARDING"));
    assert!(html.contains("<h1>Verify domain</h1>"));
    assert!(html.contains(r#"<option value="dns-txt">DNS TXT record</option>"#));
    assert!(html.contains(r#"<option value="email">Email to an address at the domain</option>"#));
    assert!(html.contains(r#"hx-post="/onboarding/verify-domain""#));
    // No record is shown before one has been minted.
    assert!(!html.contains("_indiebuild-verify"));
}

#[test]
fn step_two_shows_the_exact_record_to_publish() {
    let view = WizardView {
        domain: "example.com".into(),
        dns_token: Some("giw-verify-8f2c".into()),
        ..WizardView::default()
    };
    let html = render(OnboardingStep::VerifyDomain, &view);

    assert!(html.contains("_indiebuild-verify.example.com"));
    assert!(html.contains("giw-verify-8f2c"));
    assert!(html.contains("TXT"));
}

#[test]
fn step_three_bounds_the_seat_count() {
    let html = render(OnboardingStep::AllocateSeats, &WizardView::default());

    assert!(html.contains("03 / ONBOARDING"));
    assert!(html.contains("<h1>Allocate seats</h1>"));
    assert!(html.contains(r#"type="number""#));
    assert!(html.contains(r#"min="1" max="500""#));
    assert!(html.contains(r#"hx-post="/onboarding/seats""#));
}

#[test]
fn step_four_takes_a_csv_paste_and_a_default_role() {
    let html = render(OnboardingStep::InviteMembers, &WizardView::default());

    assert!(html.contains("04 / ONBOARDING"));
    assert!(html.contains("<h1>Invite members</h1>"));
    assert!(html.contains("<textarea"));
    assert!(html.contains(r#"name="invites""#));
    assert!(html.contains(r#"<option value="member""#));
    assert!(html.contains(r#"<option value="owner""#));
    assert!(html.contains(r#"hx-post="/onboarding/invite""#));
}

#[test]
fn step_four_reports_every_row_of_a_paste() {
    use gha_indie_worker_web_server::data::InviteOutcome;

    let view = WizardView {
        invites: vec![
            InviteOutcome::accepted("alex@example.com"),
            InviteOutcome::rejected("not-an-address", "not an email address"),
            InviteOutcome::rejected("alex@example.com", "listed more than once"),
        ],
        ..WizardView::default()
    };
    let html = render(OnboardingStep::InviteMembers, &view);

    assert!(html.contains("alex@example.com"));
    assert!(html.contains("not-an-address"));
    assert!(html.contains("not an email address"));
    assert!(html.contains("listed more than once"));
    assert_eq!(html.matches("not sent").count(), 2);
}

#[test]
fn step_five_links_to_billing_without_collecting_a_card() {
    let view = WizardView {
        billing_url: Some("https://billing.example/checkout".into()),
        ..WizardView::default()
    };
    let html = render(OnboardingStep::Billing, &view);

    assert!(html.contains("05 / ONBOARDING"));
    assert!(html.contains("https://billing.example/checkout"));
    assert!(
        !html.contains(r#"name="card"#),
        "card details must never be collected here"
    );
    assert!(!html.contains("cvc"));
}

#[test]
fn step_five_offers_a_way_forward_even_without_a_billing_link() {
    let html = render(OnboardingStep::Billing, &WizardView::default());
    assert!(html.contains("Review seats first"));
    assert!(html.contains("/onboarding/done"));
}

#[test]
fn step_six_closes_the_wizard() {
    let html = render(OnboardingStep::Done, &WizardView::default());

    assert!(html.contains("06 / ONBOARDING"));
    assert!(html.contains("Your organization is ready"));
    assert!(html.contains(r#"href="/members""#));
    assert!(html.contains("https://app.indiebuild.dev/dashboard"));
}

#[test]
fn every_step_shows_the_rail_with_exactly_one_current_entry() {
    for step in OnboardingStep::ALL {
        let html = render(step, &WizardView::default());
        assert_eq!(
            html.matches(r#"data-state="current""#).count(),
            1,
            "{step:?} does not mark exactly one current step"
        );
        for other in OnboardingStep::ALL {
            assert!(html.contains(other.title()), "{step:?} rail is missing {other:?}");
        }
    }
}

#[test]
fn a_notice_is_rendered_above_the_form_and_escaped() {
    let view = WizardView {
        notice: Some("<img src=x onerror=alert(1)>".into()),
        ..WizardView::default()
    };
    let html = render(OnboardingStep::CreateOrg, &view);

    assert!(html.contains("&lt;img"));
    assert!(!html.contains("<img"));
}

#[tokio::test]
async fn every_step_is_reachable_at_its_own_url_on_the_org_host() {
    let state = state();
    for step in OnboardingStep::ALL {
        let (status, body) = status_and_text(get(&state, ORG_HOST, &step.path()).await).await;
        assert_eq!(status, StatusCode::OK, "{step:?} is not reachable");
        assert!(body.contains(step.title()), "{step:?} does not name itself");
        assert!(body.contains(r#"data-surface="org""#));
    }
}

#[tokio::test]
async fn the_wizard_entry_point_starts_at_the_first_step() {
    let response = get(&state(), ORG_HOST, "/onboarding").await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers().get("location").and_then(|value| value.to_str().ok()),
        Some("/onboarding/create")
    );
}

#[tokio::test]
async fn an_unknown_step_slug_is_a_404() {
    let response = get(&state(), ORG_HOST, "/onboarding/steal-everything").await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_wizard_is_not_served_on_the_personal_host() {
    let response = get(&state(), common::USER_HOST, "/onboarding/create").await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
