//! Read-only release candidate observations. A successful run is not a deployment approval.
use axum::extract::{Query, State};
use axum::http::{header, HeaderValue};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Extension, Json, Router};
use maud::{html, Markup};
use serde::Deserialize;

use crate::data::models::RunSummary;
use crate::error::WebError;
use crate::hosts::{common, Surface};
use crate::middleware::RequestCtx;
use crate::state::AppState;
use crate::ui::layout::PageMeta;

pub const MAX_RUNS: u64 = 50;

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseQuery {
    pub repository: Option<String>,
    pub limit: Option<u64>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/releases", get(page))
        .route("/releases.json", get(export))
        .with_state(state)
}

fn access(context: &RequestCtx) -> Result<(), WebError> {
    if !matches!(context.surface, Surface::App | Surface::Mobile) {
        return Err(WebError::NotFound);
    }
    if context.actor.is_none() || context.bearer().is_none() {
        return Err(WebError::Unauthenticated);
    }
    Ok(())
}

fn safe_repository(value: &str) -> bool {
    let parts: Vec<_> = value.split('/').collect();
    value.len() <= 200
        && parts.len() == 2
        && parts.iter().all(|part| {
            !part.is_empty()
                && *part != "."
                && *part != ".."
                && part.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        })
}

fn valid_run(run: &RunSummary) -> bool {
    !run.id.is_empty()
        && run.id.len() <= 128
        && run.id.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        && safe_repository(&run.repository)
        && run.revision.len() == 40
        && run.revision.bytes().all(|b| b.is_ascii_hexdigit())
        && run.workflow_path.len() <= 512
        && run.status.len() <= 32
        && run.created_at.len() <= 64
        && run.duration.as_ref().is_none_or(|s| s.len() <= 64)
        && run.profile.as_ref().is_none_or(|s| s.len() <= 128)
}

fn select(runs: Vec<RunSummary>, query: &ReleaseQuery) -> Result<Vec<RunSummary>, WebError> {
    let limit = query.limit.unwrap_or(MAX_RUNS);
    if !(1..=MAX_RUNS).contains(&limit)
        || query.repository.as_deref().is_some_and(|s| !safe_repository(s))
    {
        return Err(WebError::BadRequest("Use a repository in owner/name form and a limit from 1 to 50."));
    }
    if runs.len() > MAX_RUNS as usize || runs.iter().any(|run| !valid_run(run)) {
        return Err(WebError::Unavailable);
    }
    Ok(runs
        .into_iter()
        .filter(|run| query.repository.as_ref().is_none_or(|repo| repo == &run.repository))
        .take(limit as usize)
        .collect())
}

async fn load(context: &RequestCtx, state: &AppState, query: &ReleaseQuery) -> Result<Vec<RunSummary>, WebError> {
    access(context)?;
    // Validate before I/O. Never use the optional direct-DB avenue here: the API
    // must authorize the caller's bearer, including its organization scope.
    select(Vec::new(), query)?;
    let runs = state.repo.api().runs(context.bearer(), MAX_RUNS).await?;
    select(runs, query)
}

fn private(mut response: Response) -> Response {
    response.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("private, no-store"));
    response.headers_mut().append(header::VARY, HeaderValue::from_static("Cookie, Authorization, Host"));
    response
}

async fn page(
    Extension(context): Extension<RequestCtx>,
    State(state): State<AppState>,
    Query(query): Query<ReleaseQuery>,
) -> Response {
    let result = load(&context, &state, &query).await.map(|runs| markup(&runs, &query));
    let meta = PageMeta::new("CI/CD releases", "Build observations and release evidence").active("releases");
    private(common::render_result(&context, &meta, result))
}

async fn export(
    Extension(context): Extension<RequestCtx>,
    State(state): State<AppState>,
    Query(query): Query<ReleaseQuery>,
) -> Response {
    private(match load(&context, &state, &query).await {
        Ok(runs) => Json(serde_json::json!({
            "schemaVersion": 1,
            "source": "authenticated-api-run-summaries",
            "coverage": "latest-50-runs",
            "readOnly": true,
            "promotionAllowed": false,
            "evidenceState": "not-collected",
            "runs": runs
        })).into_response(),
        Err(error) => error.into_response(),
    })
}

pub fn markup(runs: &[RunSummary], query: &ReleaseQuery) -> Markup {
    html! {
        section class="section" {
            p class="eyebrow" { "Release control / read-only observations" }
            h1 { "CI/CD release dashboard" }
            p { "Latest 50 authorized runs. Repository filters apply within that window, not the complete history." }
            p { a href="/dashboard" { "Overview" } " / " a href="/runs" { "Run details and logs" } }
            form method="get" action="/releases" {
                label for="release-repository" { "Repository (owner/name)" }
                input id="release-repository" name="repository" maxlength="200" required
                    value=(query.repository.as_deref().unwrap_or(""));
                button type="submit" { "Filter" }
                " " a href="/releases" { "Clear filter" }
            }
        }
        section class="section" aria-label="Release evidence boundaries" {
            h2 { "Evidence, not optimistic promotion" }
            p role="status" { "Build status is reported below. Required test execution, artifact attestations, environment approvals, and deployment receipts have not been collected by this view. Promotion is disabled." }
            p { "Databricks and Snowflake packages are read-only analytics adapters. Marketplace publication and installation are separate verification gates." }
        }
        section class="section" {
            h2 { (runs.len()) " observed candidates" }
            @if runs.is_empty() {
                p { "No runs matched this authorized window. This is not evidence of successful CI or deployment." }
            } @else {
                table {
                    caption { "Reported build observations — release readiness remains unverified" }
                    thead { tr { th scope="col" { "Repository" } th scope="col" { "Commit" }
                        th scope="col" { "Workflow / profile" } th scope="col" { "Reported status" }
                        th scope="col" { "Created" } th scope="col" { "Release evidence" } } }
                    tbody {
                        @for run in runs {
                            tr {
                                td { a href=(run.href()) { (&run.repository) } }
                                td { code title=(&run.revision) { (run.short_revision()) } }
                                td { (&run.workflow_path) " / " (run.profile.as_deref().unwrap_or("unreported")) }
                                td { (&run.status) }
                                td { (&run.created_at) }
                                td { "Not collected — no promotion authority" }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn run() -> RunSummary {
        RunSummary { id: "run_1".into(), repository: "gha-indie-worker/worker".into(),
            revision: "a".repeat(40), status: "success".into(), ..RunSummary::default() }
    }
    #[test]
    fn successful_run_never_implies_release_approval() {
        let html = markup(&[run()], &ReleaseQuery::default()).into_string();
        assert!(html.contains("Promotion is disabled"));
        assert!(html.contains("Not collected"));
    }
    #[test]
    fn escapes_untrusted_labels() {
        let mut row = run(); row.workflow_path = "<script>alert(1)</script>".into();
        let html = markup(&[row], &ReleaseQuery::default()).into_string();
        assert!(!html.contains("<script>")); assert!(html.contains("&lt;script&gt;"));
    }
    #[test]
    fn validates_filters_and_caps_without_silent_truncation() {
        for limit in [0, 51, u64::MAX] {
            assert!(select(vec![], &ReleaseQuery { limit: Some(limit), repository: None }).is_err());
        }
        assert!(select(vec![run(); 51], &ReleaseQuery::default()).is_err());
        assert!(select(vec![], &ReleaseQuery { repository: Some("../secret".into()), limit: None }).is_err());
    }
    #[test]
    fn rejects_malformed_upstream_identity() {
        let mut row = run(); row.id = "../../settings".into();
        assert!(select(vec![row], &ReleaseQuery::default()).is_err());
        let mut row = run(); row.revision = "main".into();
        assert!(select(vec![row], &ReleaseQuery::default()).is_err());
    }
    #[test]
    fn filters_exactly_in_authorized_window() {
        let result = select(vec![run()], &ReleaseQuery { repository: Some("other/repo".into()), limit: None }).unwrap();
        assert!(result.is_empty());
    }
}
