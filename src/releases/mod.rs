#![forbid(unsafe_code)]

//! Read-only release observations. A successful run is not a deployment approval.

mod status;

use std::collections::HashSet;

use axum::extract::rejection::QueryRejection;
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
    pub status: Option<String>,
}

impl ReleaseQuery {
    fn repository(&self) -> Option<&str> {
        self.repository.as_deref().filter(|value| !value.is_empty())
    }

    fn status(&self) -> &str {
        self.status.as_deref().unwrap_or("all")
    }

    fn export_href(&self) -> String {
        // Validation restricts repository to ASCII ._- and /, status to fixed
        // names, and limit to digits. None can introduce another query parameter.
        let mut href = format!(
            "/releases.json?limit={}&status={}",
            self.limit.unwrap_or(MAX_RUNS),
            self.status()
        );
        if let Some(repository) = self.repository() {
            href.push_str("&repository=");
            href.push_str(repository);
        }
        href
    }
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

fn checked_query(
    context: &RequestCtx,
    query: Result<Query<ReleaseQuery>, QueryRejection>,
) -> Result<ReleaseQuery, WebError> {
    // Preserve 400 for malformed product queries, but never reveal this route
    // on a non-product surface. Authentication still precedes all data access.
    if !matches!(context.surface, Surface::App | Surface::Mobile) {
        return Err(WebError::NotFound);
    }
    query
        .map(|Query(query)| query)
        .map_err(|_| WebError::BadRequest("Invalid release filters."))
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
        || query.repository().is_some_and(|s| !safe_repository(s))
        || !matches!(query.status(), "all" | "success" | "failure" | "active" | "other")
    {
        return Err(WebError::BadRequest(
            "Use owner/name, a known status group, and a limit from 1 to 50.",
        ));
    }
    let mut identities = HashSet::new();
    if runs.len() > MAX_RUNS as usize
        || runs
            .iter()
            .any(|run| !valid_run(run) || !identities.insert((&run.repository, &run.id)))
    {
        return Err(WebError::Unavailable);
    }
    drop(identities);
    Ok(runs
        .into_iter()
        .filter(|run| query.repository().is_none_or(|repo| repo == run.repository.as_str()))
        .filter(|run| query.status() == "all" || status::classify(&run.status).as_str() == query.status())
        .take(limit as usize)
        .collect())
}

async fn load(context: &RequestCtx, state: &AppState, query: &ReleaseQuery) -> Result<Vec<RunSummary>, WebError> {
    access(context)?;
    // Validate before I/O. The API must authorize the caller's bearer and org scope.
    select(Vec::new(), query)?;
    let runs = state.repo.api().runs(context.bearer(), MAX_RUNS).await?;
    select(runs, query)
}

fn private(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("private, no-store"));
    response
        .headers_mut()
        .append(header::VARY, HeaderValue::from_static("Cookie, Authorization, Host"));
    response
}

async fn page(
    Extension(context): Extension<RequestCtx>,
    State(state): State<AppState>,
    query: Result<Query<ReleaseQuery>, QueryRejection>,
) -> Response {
    let result = match checked_query(&context, query) {
        Ok(query) => load(&context, &state, &query).await.map(|runs| markup(&runs, &query)),
        Err(error) => Err(error),
    };
    let meta = PageMeta::new("CI/CD releases", "Build observations and release evidence").active("releases");
    private(common::render_result(&context, &meta, result))
}

async fn export(
    Extension(context): Extension<RequestCtx>,
    State(state): State<AppState>,
    query: Result<Query<ReleaseQuery>, QueryRejection>,
) -> Response {
    let result = match checked_query(&context, query) {
        Ok(query) => load(&context, &state, &query).await.map(|runs| {
            let counts = status::summarize(runs.iter().map(|run| run.status.as_str()));
            Json(serde_json::json!({
                "schemaVersion": 1,
                "source": "authenticated-api-run-summaries",
                "coverage": "latest-50-runs",
                "readOnly": true,
                "promotionAllowed": false,
                "evidenceState": "not-collected",
                "filters": {"repository": query.repository(), "status": query.status(), "limit": query.limit.unwrap_or(MAX_RUNS)},
                "counts": {"total": counts.total, "success": counts.success, "failure": counts.failure, "active": counts.active, "other": counts.other},
                "runs": runs
            }))
            .into_response()
        }),
        Err(error) => Err(error),
    };
    private(result.unwrap_or_else(IntoResponse::into_response))
}

pub fn markup(runs: &[RunSummary], query: &ReleaseQuery) -> Markup {
    let counts = status::summarize(runs.iter().map(|run| run.status.as_str()));
    html! {
        section class="section" {
            p class="eyebrow" { "Release control / read-only observations" }
            h1 { "CI/CD release dashboard" }
            p { "Latest 50 authorized runs. Filters apply within that window, not the complete history. API ordering is preserved." }
            p { a href="/dashboard" { "Overview" } " / " a href="/runs" { "Run details and logs" } }
            form method="get" action="/releases" {
                label for="release-repository" { "Repository (owner/name; blank for all authorized repositories)" }
                input id="release-repository" name="repository" maxlength="200"
                    value=(query.repository().unwrap_or(""));
                label for="release-status" { "Reported status group" }
                select id="release-status" name="status" {
                    @for (value, label) in [("all", "All observations"), ("success", "Reported success"), ("failure", "Reported failure"), ("active", "Queued or running"), ("other", "Other / unknown")] {
                        option value=(value) selected[value == query.status()] { (label) }
                    }
                }
                label for="release-limit" { "Maximum displayed observations" }
                input id="release-limit" type="number" name="limit" min="1" max="50"
                    value=(query.limit.unwrap_or(MAX_RUNS));
                button type="submit" { "Apply filters" }
                " " a href="/releases" { "Clear filters" }
            }
            p { "These filters can be bookmarked in the page URL. " a href=(query.export_href()) { "Export filtered JSON" } }
        }
        section class="section" aria-label="Filtered build observations" {
            h2 { "Displayed observations" }
            dl {
                dt { "Total" } dd { (counts.total) }
                dt { "Reported success" } dd { (counts.success) }
                dt { "Reported failure" } dd { (counts.failure) }
                dt { "Queued or running" } dd { (counts.active) }
                dt { "Other / unknown" } dd { (counts.other) }
            }
            p { "Counts describe only the displayed rows after filters and limit. Completed, skipped, cancelled and unknown statuses are not inferred successes." }
        }
        section class="section" aria-label="Release evidence boundaries" {
            h2 { "Evidence, not optimistic promotion" }
            p role="status" { "Build status is reported below. Required test execution, artifact attestations, environment approvals, and deployment receipts have not been collected by this view. Promotion is disabled." }
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
                        th scope="col" { "Created" } th scope="col" { "Inspect" } } }
                    tbody {
                        @for run in runs {
                            tr {
                                td { a href=(run.href()) { (&run.repository) } }
                                td { code title=(&run.revision) { (run.short_revision()) } }
                                td { (&run.workflow_path) " / " (run.profile.as_deref().unwrap_or("unreported")) }
                                td { (&run.status) }
                                td { (&run.created_at) }
                                td { details {
                                    summary { "Inspect candidate" }
                                    p { "Full commit: " code { (&run.revision) } }
                                    p { "Reported duration: " (run.duration.as_deref().unwrap_or("unreported")) }
                                    p { "Not collected — no promotion authority" }
                                    a href=(run.href()) { "Open run and logs" }
                                } }
                            }
                        }
                    }
                }
            }
        }
        section class="section" aria-label="Analytics integrations" {
            h2 { "Analytics integrations" }
            p { "Inspect capabilities and permissions before installation. These are not installed marketplace listings." }
            details {
                summary { "Databricks — development package" }
                p { "Read-only SQL warehouse dashboard. Requires per-user authorization and SELECT access to an approved consumer view. No deployment controls or service-principal fallback. Installation and marketplace publication remain unverified." }
            }
            details {
                summary { "Snowflake — planned Native App" }
                p { "Consumer-approved SELECT-only view reference and isolated application role are required. Native App packaging, account installation and marketplace publication are not complete. No install action is offered." }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run() -> RunSummary {
        RunSummary {
            id: "run_1".into(),
            repository: "gha-indie-worker/worker".into(),
            revision: "a".repeat(40),
            status: "success".into(),
            ..RunSummary::default()
        }
    }

    #[test]
    fn successful_run_never_implies_release_approval() {
        let html = markup(&[run()], &ReleaseQuery::default()).into_string();
        assert!(html.contains("Promotion is disabled"));
        assert!(html.contains("Not collected"));
    }

    #[test]
    fn escapes_untrusted_labels() {
        let mut row = run();
        row.workflow_path = "<script>alert(1)</script>".into();
        let html = markup(&[row], &ReleaseQuery::default()).into_string();
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn rejects_invalid_limits_and_oversized_windows() {
        for limit in [0, 51, u64::MAX] {
            let query = ReleaseQuery {
                limit: Some(limit),
                ..ReleaseQuery::default()
            };
            assert!(select(vec![], &query).is_err());
        }
        assert!(select(vec![run(); 51], &ReleaseQuery::default()).is_err());
    }

    #[test]
    fn rejects_malformed_upstream_identity_and_duplicates() {
        for (id, revision) in [("../../settings", "a".repeat(40)), ("run_1", "main".into())] {
            let row = RunSummary {
                id: id.into(),
                revision,
                ..run()
            };
            assert!(select(vec![row], &ReleaseQuery::default()).is_err());
        }
        assert!(select(vec![run(), run()], &ReleaseQuery::default()).is_err());
    }

    #[test]
    fn filters_exactly_in_authorized_window() {
        let query = ReleaseQuery {
            repository: Some("other/repo".into()),
            ..ReleaseQuery::default()
        };
        assert!(select(vec![run()], &query).unwrap().is_empty());
    }

    #[test]
    fn rejects_bad_repository_or_status_before_io() {
        for repository in ["../secret", "a/b?token=x", "a/b&status=success"] {
            let query = ReleaseQuery {
                repository: Some(repository.into()),
                ..ReleaseQuery::default()
            };
            assert!(select(vec![], &query).is_err());
        }
        let query = ReleaseQuery {
            status: Some("approved".into()),
            ..ReleaseQuery::default()
        };
        assert!(select(vec![], &query).is_err());
    }

    #[test]
    fn blank_repository_means_all_not_a_broken_form() {
        let query = ReleaseQuery {
            repository: Some(String::new()),
            ..ReleaseQuery::default()
        };
        assert_eq!(select(vec![run()], &query).unwrap().len(), 1);
    }

    #[test]
    fn status_filter_never_upgrades_completed_to_success() {
        let row = RunSummary {
            status: "completed".into(),
            ..run()
        };
        let query = ReleaseQuery {
            status: Some("success".into()),
            ..ReleaseQuery::default()
        };
        assert!(select(vec![row], &query).unwrap().is_empty());
    }

    #[test]
    fn inspection_and_export_preserve_exact_commit_and_filters() {
        let query = ReleaseQuery {
            repository: Some("gha-indie-worker/worker".into()),
            status: Some("success".into()),
            limit: Some(10),
        };
        assert_eq!(
            query.export_href(),
            "/releases.json?limit=10&status=success&repository=gha-indie-worker/worker"
        );
        let html = markup(&[run()], &query).into_string();
        assert!(html.contains(&"a".repeat(40)));
        assert!(html.contains("Other / unknown"));
        assert!(html.contains("not installed marketplace listings"));
    }
}
