#![forbid(unsafe_code)]

//! `app.indiebuild.dev` — the primary product surface.
//!
//! Signed out, `/` is the **continuation of the marketing site**: the same mark,
//! palette and eyebrow rhythm as `gha-indie-worker.github.io`, and exactly two
//! entry points, stated plainly —
//!
//! * **Sign in to your organization** → `https://org.indiebuild.dev/login`
//! * **Personal account** → `https://user.indiebuild.dev/login`
//!
//! Signed in, the same URL is the product: runs, a live run detail with a log
//! stream, workers, plans and settings. One address, two states, no interstitial.

use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Extension, Form, Router};
use maud::{html, Markup};
use serde::Deserialize;

use crate::data::models::{PlanSummary, RunDetail, RunSummary, WorkerSummary};
use crate::error::WebError;
use crate::hosts::{common, Surface};
use crate::middleware::RequestCtx;
use crate::state::AppState;
use crate::ui::chat_widget::ChatAudience;
use crate::ui::layout::PageMeta;
use crate::ui::{components, forms, loader, nav, tables};

/// How many runs a list page shows.
pub const RUN_PAGE_SIZE: u64 = 50;

/// The `app.` router. `m.` mounts the same routes with a compact shell.
#[must_use]
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(home))
        .route("/dashboard", get(dashboard))
        .route("/runs", get(runs))
        .route("/runs/{run_id}", get(run_detail))
        .route("/runs/{run_id}/log", get(run_log))
        .route("/runs/{run_id}/rerun", post(rerun))
        .route("/workers", get(workers))
        .route("/plans", get(plans))
        .route("/settings", get(settings))
        .merge(common::routes())
}

#[must_use]
pub fn router(state: AppState) -> Router {
    routes().with_state(state)
}

// ---- pages -----------------------------------------------------------------

async fn home(Extension(context): Extension<RequestCtx>, State(state): State<AppState>) -> Response {
    if context.actor.is_some() {
        return dashboard(Extension(context), State(state)).await;
    }
    let meta = PageMeta::new(
        "Run Actions on your own machines",
        "GHA Indie Worker makes the self-hosted runner contract inspectable: native contracts, fleet certification, visible eligibility.",
    )
    .active("access")
    .with_release_prefetch()
    .with_chat(ChatAudience::Visitor);
    common::render(&context, &meta, marketing_home(&context))
}

/// The signed-out home. Deliberately mirrors the Astro site's section rhythm:
/// hero → thesis → principles → **access** (the two entry points) → closing.
#[must_use]
pub fn marketing_home(context: &RequestCtx) -> Markup {
    let page = context.page();
    let org_login = format!("{}/login", page.origin(Surface::Org));
    let user_login = format!("{}/login", page.origin(Surface::User));
    html! {
        section class="section" {
            p class="eyebrow" { "Native self-hosted runner contracts" }
            h1 { "Run Actions on your own machines. Keep the contract native." }
            p class="lede" {
                "A self-hosted runner is more than a registered process. It is an operating-system \
                 boundary, a toolchain, a trust decision, and a reproducibility claim. GHA Indie \
                 Worker makes that contract inspectable."
            }
            div class="row" { (loader::open_app_link(&page, "Open the app")) }
        }

        section class="section" id="principles" {
            p class="section-number" { "01 / PRINCIPLES" }
            h2 { "What the system protects." }
            div class="grid grid-3" {
                (components::card("Native contracts", html! {
                    h3 { "Describe the host" }
                    p { "The operating system, tools, isolation and lifecycle a workflow can rely on — written down, not implied." }
                }))
                (components::card("Fleet certification", html! {
                    h3 { "Test from outside" }
                    p { "Runners are exercised from an external organization, as a real consumer would." }
                }))
                (components::card("Visible eligibility", html! {
                    h3 { "Evidence, not labels" }
                    p { "Jobs match machines through evidence instead of optimistic labels." }
                }))
            }
        }

        section class="section" id="method" {
            p class="section-number" { "02 / METHOD" }
            h2 { "A visible sequence." }
            (components::step_indicator(&[
                (1, "Inspect — measure the runner against its declared contract", components::StepState::Done),
                (2, "Certify — exercise it from the external test boundary", components::StepState::Done),
                (3, "Dispatch — accept only work the verified environment can satisfy", components::StepState::Done),
            ]))
        }

        section class="section" id="access" {
            p class="section-number" { "03 / ACCESS" }
            h2 { "One product. Clear account boundaries." }
            div class="entry-points" {
                article class="entry-point" {
                    span class="label" { "ORGANIZATIONS" }
                    h3 { "Sign in to your organization" }
                    p { "Shared policy, membership, seats and operational work for a team. Organization accounts live on their own host." }
                    a class="button" href=(org_login) { "Sign in to your organization" }
                }
                article class="entry-point" {
                    span class="label" { "INDIVIDUALS" }
                    h3 { "Personal account" }
                    p { "Your own runs, workers and API tokens. Start here if nobody has invited you to an organization yet." }
                    a class="button secondary" href=(user_login) { "Personal account" }
                }
            }
        }

        section class="section" {
            h2 { "Owning the machine should increase clarity, not create a hidden platform." }
            p { a class="text-link" href="https://github.com/gha-indie-worker" { "github.com/gha-indie-worker" } }
        }
    }
}

async fn dashboard(Extension(context): Extension<RequestCtx>, State(state): State<AppState>) -> Response {
    if context.actor.is_none() {
        return crate::session::login_redirect(context.surface, &context.base_domain, Some("/dashboard"));
    }
    let runs = state
        .repo
        .runs(context.bearer(), context.org_id(), 10)
        .await
        .unwrap_or_default();
    let workers = state
        .repo
        .workers(context.bearer(), context.org_id())
        .await
        .unwrap_or_default();
    let meta = PageMeta::new("Dashboard", "Recent runs and worker health")
        .active("runs")
        .with_chat(ChatAudience::Customer);
    let body = html! {
        section class="section" {
            p class="eyebrow" { "Overview" }
            h1 { "Dashboard" }
            div class="grid grid-3" {
                (components::stat("Runs (recent)", &runs.len().to_string(), "Newest first"))
                (components::stat("Workers", &workers.len().to_string(), &format!("{} certified", workers.iter().filter(|w| w.certified).count())))
                (components::stat("Read avenue", state.repo.read_source().as_str(), "Canonical database first, api-server otherwise"))
            }
        }
        (components::section("01 / RUNS", "Recent runs", run_table(&runs)))
        (components::section("02 / WORKERS", "Fleet", worker_table(&workers)))
    };
    common::render(&context, &meta, body)
}

#[derive(Debug, Deserialize)]
pub struct RunsQuery {
    #[serde(default)]
    pub limit: Option<u64>,
}

async fn runs(
    Extension(context): Extension<RequestCtx>,
    State(state): State<AppState>,
    Query(query): Query<RunsQuery>,
) -> Response {
    if context.actor.is_none() {
        return crate::session::login_redirect(context.surface, &context.base_domain, Some("/runs"));
    }
    let limit = query.limit.unwrap_or(RUN_PAGE_SIZE).clamp(1, RUN_PAGE_SIZE);
    let meta = PageMeta::new("Runs", "Every run, newest first")
        .active("runs")
        .with_chat(ChatAudience::Customer);
    let body = state
        .repo
        .runs(context.bearer(), context.org_id(), limit)
        .await
        .map(|runs| components::section("", "Runs", run_table(&runs)));
    common::render_result(&context, &meta, body)
}

async fn run_detail(
    Extension(context): Extension<RequestCtx>,
    State(state): State<AppState>,
    Path(run_id): Path<String>,
) -> Response {
    if context.actor.is_none() {
        return crate::session::login_redirect(context.surface, &context.base_domain, Some("/runs"));
    }
    if !crate::ws::is_safe_id(&run_id) {
        return common::error_page(&context, &WebError::NotFound);
    }
    let meta = PageMeta::new("Run", "Live run detail and log stream")
        .active("runs")
        .with_chat(ChatAudience::Customer);
    let body = state
        .repo
        .run(context.bearer(), &run_id)
        .await
        .map(|detail| run_detail_body(&context, &detail));
    common::render_result(&context, &meta, body)
}

/// The htmx polling fallback for the log container.
async fn run_log(
    Extension(context): Extension<RequestCtx>,
    State(state): State<AppState>,
    Path(run_id): Path<String>,
) -> Response {
    if context.actor.is_none() {
        return WebError::Unauthenticated.into_response();
    }
    if !crate::ws::is_safe_id(&run_id) {
        return WebError::NotFound.into_response();
    }
    match state.repo.run_log(context.bearer(), &run_id, None).await {
        Ok(lines) => components::log_lines(&lines).into_response(),
        Err(error) => error.into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct RerunForm {
    #[serde(default)]
    pub csrf_token: Option<String>,
}

/// Writes always go to the api-server, with the actor's bearer forwarded.
async fn rerun(
    Extension(context): Extension<RequestCtx>,
    State(state): State<AppState>,
    Path(run_id): Path<String>,
    Form(form): Form<RerunForm>,
) -> Response {
    if let Err(error) = context.verify_form_csrf(&state.csrf, form.csrf_token.as_deref()) {
        return error.into_response();
    }
    if context.actor.is_none() {
        return WebError::Unauthenticated.into_response();
    }
    if !crate::ws::is_safe_id(&run_id) {
        return WebError::NotFound.into_response();
    }
    match state.repo.api().rerun(context.bearer(), &run_id).await {
        Ok(run) => Redirect::to(&run.href()).into_response(),
        Err(error) => error.into_response(),
    }
}

async fn workers(Extension(context): Extension<RequestCtx>, State(state): State<AppState>) -> Response {
    if context.actor.is_none() {
        return crate::session::login_redirect(context.surface, &context.base_domain, Some("/workers"));
    }
    let meta = PageMeta::new("Workers", "Registered machines and their certification")
        .active("workers")
        .with_chat(ChatAudience::Customer);
    let body = state
        .repo
        .workers(context.bearer(), context.org_id())
        .await
        .map(|workers| components::section("", "Workers", worker_table(&workers)));
    common::render_result(&context, &meta, body)
}

async fn plans(Extension(context): Extension<RequestCtx>, State(state): State<AppState>) -> Response {
    let meta = PageMeta::new("Plans", "Fixed build profiles the independent lane can execute")
        .active("plans")
        .with_chat(ChatAudience::Customer);
    let body = state
        .repo
        .plans(context.bearer())
        .await
        .map(|plans| components::section("", "Build profiles", plan_table(&plans)));
    common::render_result(&context, &meta, body)
}

async fn settings(Extension(context): Extension<RequestCtx>) -> Response {
    if context.actor.is_none() {
        return crate::session::login_redirect(context.surface, &context.base_domain, Some("/settings"));
    }
    let page = context.page();
    let meta = PageMeta::new("Settings", "Account and surface settings")
        .active("settings")
        .with_chat(ChatAudience::Customer);
    let body = html! {
        section class="section" {
            p class="eyebrow" { "Settings" }
            h1 { "Settings" }
            div class="grid grid-2" {
                (components::card("PERSONAL", html! {
                    h3 { "Your account" }
                    p { "API tokens, two-factor methods and passkeys live on your personal host." }
                    a class="button secondary" href=(format!("{}/security", page.origin(Surface::User))) { "Open personal settings" }
                }))
                (components::card("ORGANIZATION", html! {
                    h3 { "Your organization" }
                    p { "Members, roles, seats, audit and single sign-on live on the organization host." }
                    a class="button secondary" href=(format!("{}/members", page.origin(Surface::Org))) { "Open organization settings" }
                }))
            }
        }
    };
    common::render(&context, &meta, body)
}

// ---- fragments -------------------------------------------------------------

#[must_use]
pub fn run_table(runs: &[RunSummary]) -> Markup {
    let rows = runs
        .iter()
        .map(|run| {
            tables::row(vec![
                tables::link_cell(&run.href(), &run.repository),
                tables::cell(run.short_revision()),
                tables::cell(&run.workflow_path),
                tables::markup_cell(components::status_badge(&run.status)),
                tables::numeric_cell(run.duration.as_deref().unwrap_or("—")),
                tables::cell(&run.created_at),
            ])
        })
        .collect();
    tables::table(
        "Runs",
        &[
            tables::Column::text("Repository"),
            tables::Column::text("Revision"),
            tables::Column::text("Workflow"),
            tables::Column::text("Status"),
            tables::Column::numeric("Duration"),
            tables::Column::text("Started"),
        ],
        rows,
    )
}

#[must_use]
pub fn worker_table(workers: &[WorkerSummary]) -> Markup {
    let rows = workers
        .iter()
        .map(|worker| {
            tables::row(vec![
                tables::cell(&worker.name),
                tables::cell(&format!("{} {}", worker.os, worker.arch)),
                tables::markup_cell(components::status_badge(&worker.status)),
                tables::markup_cell(if worker.certified {
                    components::badge("certified", components::Tone::Ok)
                } else {
                    components::badge("unverified", components::Tone::Warn)
                }),
                tables::cell(&worker.profiles.join(", ")),
                tables::cell(worker.last_seen_at.as_deref().unwrap_or("—")),
            ])
        })
        .collect();
    tables::table(
        "Workers",
        &[
            tables::Column::text("Name"),
            tables::Column::text("Platform"),
            tables::Column::text("Status"),
            tables::Column::text("Contract"),
            tables::Column::text("Profiles"),
            tables::Column::text("Last seen"),
        ],
        rows,
    )
}

#[must_use]
pub fn plan_table(plans: &[PlanSummary]) -> Markup {
    let rows = plans
        .iter()
        .map(|plan| {
            tables::row(vec![
                tables::cell(&plan.name),
                tables::cell(&plan.description),
                tables::cell(&plan.evidence.join(", ")),
                tables::markup_cell(if plan.enabled {
                    components::badge("enabled", components::Tone::Ok)
                } else {
                    components::badge("disabled", components::Tone::Neutral)
                }),
            ])
        })
        .collect();
    tables::table(
        "Build profiles",
        &[
            tables::Column::text("Profile"),
            tables::Column::text("Description"),
            tables::Column::text("Workflow evidence"),
            tables::Column::text("State"),
        ],
        rows,
    )
}

#[must_use]
pub fn run_detail_body(context: &RequestCtx, detail: &RunDetail) -> Markup {
    let run = &detail.summary;
    let live = run.is_live();
    let rerun_path = format!("/runs/{}/rerun", run.id);
    let run_href = run.href();
    let trail = [("Runs", "/runs"), (run.repository.as_str(), run_href.as_str())];
    html! {
        section class="section" {
            (nav::breadcrumbs(&trail))
            p class="eyebrow" { (run.workflow_path) }
            h1 { (run.repository) }
            div class="row" {
                (components::status_badge(&run.status))
                span class="text-link" { (run.short_revision()) }
                @if let Some(profile) = &run.profile {
                    (components::badge(profile, components::Tone::Neutral))
                }
                (forms::form(
                    &forms::FormAction::post(rerun_path.as_str()),
                    &context.csrf_token,
                    html! { button type="submit" class="button secondary" { "Re-run" } },
                ))
            }
        }
        @if !detail.exclusions.is_empty() {
            section class="section" {
                (components::alert(
                    components::Tone::Warn,
                    "Some workflow behaviour is not supported by the independent lane",
                    "These jobs are reported explicitly rather than approximated. They still receive an ARC classification.",
                ))
                ul {
                    @for exclusion in &detail.exclusions {
                        li { (exclusion) }
                    }
                }
            }
        }
        (components::section("01 / JOBS", "Jobs", job_table(detail)))
        section class="section" {
            p class="section-number" { "02 / LOG" }
            h2 { "Output" }
            (components::log_stream(&run.id, &detail.log_tail, live))
        }
    }
}

fn job_table(detail: &RunDetail) -> Markup {
    let rows = detail
        .jobs
        .iter()
        .map(|job| {
            tables::row(vec![
                tables::cell(&job.name),
                tables::markup_cell(components::status_badge(&job.status)),
                tables::cell(job.profile.as_deref().unwrap_or("—")),
                tables::cell(job.lane.as_deref().unwrap_or("independent")),
                tables::numeric_cell(job.duration.as_deref().unwrap_or("—")),
            ])
        })
        .collect();
    tables::table(
        "Jobs",
        &[
            tables::Column::text("Job"),
            tables::Column::text("Status"),
            tables::Column::text("Profile"),
            tables::Column::text("Lane"),
            tables::Column::numeric("Duration"),
        ],
        rows,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::middleware::RequestCtx;

    fn context() -> RequestCtx {
        RequestCtx {
            surface: Surface::App,
            host: "app.indiebuild.dev".into(),
            base_domain: "indiebuild.dev".into(),
            path: "/".into(),
            nonce: "n0nce".into(),
            csrf_token: "tok3n".into(),
            csrf: crate::csrf::CsrfCheck::NotRequired,
            actor: None,
            is_htmx: false,
            subject: "anonymous".into(),
            release_manifest_url: None,
            chat_enabled: true,
        }
    }

    #[test]
    fn the_marketing_home_states_both_entry_points_verbatim() {
        let html = marketing_home(&context()).into_string();
        assert!(html.contains("https://org.indiebuild.dev/login"));
        assert!(html.contains("https://user.indiebuild.dev/login"));
        assert!(html.contains("Sign in to your organization"));
        assert!(html.contains("Personal account"));
    }

    #[test]
    fn the_marketing_home_keeps_the_sites_section_rhythm() {
        let html = marketing_home(&context()).into_string();
        assert!(html.contains("01 / PRINCIPLES"));
        assert!(html.contains("02 / METHOD"));
        assert!(html.contains("03 / ACCESS"));
    }

    #[test]
    fn the_marketing_home_only_warms_the_release_on_intent() {
        let html = marketing_home(&context()).into_string();
        assert!(html.contains("mouseenter once, focus once"));
        assert!(!html.contains(r#"rel="prefetch""#));
    }

    #[test]
    fn a_live_run_streams_and_a_finished_run_polls() {
        let mut detail = RunDetail::default();
        detail.summary.id = "run_1".into();
        detail.summary.status = "running".into();
        let live = run_detail_body(&context(), &detail).into_string();
        assert!(live.contains(r#"ws-connect="/ws?run_id=run_1""#));

        detail.summary.status = "succeeded".into();
        let done = run_detail_body(&context(), &detail).into_string();
        assert!(done.contains(r#"hx-get="/runs/run_1/log""#));
    }

    #[test]
    fn the_rerun_control_is_a_csrf_protected_post() {
        let mut detail = RunDetail::default();
        detail.summary.id = "run_1".into();
        let html = run_detail_body(&context(), &detail).into_string();
        assert!(html.contains(r#"hx-post="/runs/run_1/rerun""#));
        assert!(html.contains(r#"name="csrf_token" value="tok3n""#));
    }

    #[test]
    fn reported_exclusions_are_shown_not_hidden() {
        let mut detail = RunDetail::default();
        detail.exclusions = vec!["job `mac` uses macOS native execution".into()];
        let html = run_detail_body(&context(), &detail).into_string();
        assert!(html.contains("not supported by the independent lane"));
        assert!(html.contains("macOS native execution"));
    }

    #[test]
    fn tables_render_every_row_and_escape_their_content() {
        let runs = vec![RunSummary {
            id: "run_1".into(),
            repository: "<b>owner/repo</b>".into(),
            revision: "0".repeat(40),
            workflow_path: ".github/workflows/ci.yml".into(),
            status: "succeeded".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            duration: None,
            profile: None,
        }];
        let html = run_table(&runs).into_string();
        assert!(html.contains("&lt;b&gt;owner/repo&lt;/b&gt;"));
        assert!(html.contains(r#"href="/runs/run_1""#));
        assert!(html.contains(r#"<td class="numeric">—</td>"#));
    }
}
