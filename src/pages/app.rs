#![forbid(unsafe_code)]
//! `app.` and `m.` — the signed-in shell.
//!
//! Both surfaces render the same pages from the same data; `m.` differs in chrome (bottom
//! navigation, larger targets, no live-log island) rather than in content, so there is no second
//! set of templates to keep in step.
//!
//! The run detail page is the only one with an interactive island. It is built so that the island
//! is an *upgrade*: the lines already recorded for the run are rendered server-side into the same
//! list the socket appends to, so the page is complete and readable before any script runs, and a
//! visitor with no script sees the log up to the moment they loaded it.

use maud::{html, Markup};

use crate::persistence::{Run, Runner};
use crate::present::{status_badge, Page, RunStatus};
use crate::state::{AppState, Ctx};

/// Render an `app.`/`m.` page.
#[must_use]
pub fn render(state: &AppState, ctx: &Ctx, page: &Page) -> Markup {
    match page {
        Page::AppDashboard => dashboard(state, ctx),
        Page::AppRuns => runs_page(state),
        Page::AppRunDetail(id) => match state.store.run(id) {
            Some(run) => run_detail(state, ctx, &run),
            None => super::not_found(ctx),
        },
        Page::AppRunners => runners_page(state),
        Page::AppCaches => caches_page(),
        Page::AppSettings => settings_page(state, ctx),
        _ => super::not_found(ctx),
    }
}

/// The one inline script in the product: the log tail's configuration, under the page's CSP nonce.
///
/// It is a single assignment of a JSON literal, built from values that have already been
/// validated — the run identifier by [`crate::present::is_run_id`], the host by
/// [`crate::policy::websocket_origin`] — so there is nothing here for a run name to escape into.
#[must_use]
pub fn inline_script(ctx: &Ctx, page: &Page, state: &AppState) -> Option<String> {
    let Page::AppRunDetail(id) = page else {
        return None;
    };
    if !crate::present::is_run_id(id) {
        return None;
    }
    let run = state.store.run(id)?;
    if ctx.face.is_small_screen() {
        // The mobile shell deliberately has no live island: a phone on a train is the worst place
        // to hold a socket open, and the page is complete without it.
        return None;
    }
    let socket = format!(
        "{}/ws",
        crate::policy::websocket_origin(&ctx.host, state.config.cookies_are_secure())?
    );
    let from = state.store.log_lines(id).len();
    Some(format!(
        "window.__giw={{\"stream\":\"{}\",\"from\":{from},\"socket\":\"{socket}\"}};",
        run.log_stream()
    ))
}

fn dashboard(state: &AppState, ctx: &Ctx) -> Markup {
    let runs = state.store.runs();
    let runners = state.store.runners();
    let online = runners.iter().filter(|runner| runner.online).count();
    let failing = runs
        .iter()
        .filter(|run| run.status == RunStatus::Failed)
        .count();
    let name = ctx
        .session
        .as_ref()
        .map_or("there", |session| session.display_name.as_str());
    html! {
        div class="stack" {
            header class="stack-tight" {
                h1 { "Good to see you, " (name) "." }
                p class="lede" {
                    (online) " of " (runners.len()) " runners online. "
                    @if failing == 0 {
                        "Nothing is failing."
                    } @else {
                        (failing) " run" @if failing != 1 { "s" } " need attention."
                    }
                }
            }
            section class="card stack" aria-labelledby="recent-heading" {
                div class="row-between" {
                    h2 id="recent-heading" { "Recent runs" }
                    a class="button button-quiet" href="/runs" { "All runs" }
                }
                (run_table(runs.iter().take(3)))
            }
            section class="card stack" aria-labelledby="runner-heading" {
                div class="row-between" {
                    h2 id="runner-heading" { "Runners" }
                    a class="button button-quiet" href="/runners" { "Manage" }
                }
                (runner_table(&runners))
            }
        }
    }
}

fn runs_page(state: &AppState) -> Markup {
    let runs = state.store.runs();
    html! {
        div class="stack" {
            header class="stack-tight" {
                h1 { "Runs" }
                p class="lede" { "Every workflow run across your repositories, newest first." }
            }
            section class="card" { (run_table(runs.iter())) }
        }
    }
}

fn run_table<'a>(runs: impl Iterator<Item = &'a Run>) -> Markup {
    html! {
        div class="table-scroll" {
            table {
                caption class="visually-hidden" { "Workflow runs" }
                thead { tr {
                    th scope="col" { "Status" }
                    th scope="col" { "Run" }
                    th scope="col" { "Branch" }
                    th scope="col" { "Runner" }
                    th scope="col" { "Duration" }
                } }
                tbody {
                    @for run in runs {
                        @let badge = status_badge(run.status);
                        @let href = format!("/runs/{}", run.id);
                        tr {
                            td {
                                span class=(badge.class) {
                                    span class="glyph" aria-hidden="true" { (badge.glyph) }
                                    (badge.label)
                                }
                            }
                            td {
                                div class="stack-tight" {
                                    a href=(href) { (run.repository) " · " (run.workflow) }
                                    span class="faint" { (run.message) }
                                }
                            }
                            td class="mono faint" { (run.branch) " @ " (run.commit) }
                            td class="faint" {
                                @if let Some(runner) = &run.runner {
                                    (runner)
                                } @else {
                                    "unassigned"
                                }
                            }
                            td class="mono faint" {
                                @if let Some(seconds) = run.duration_seconds {
                                    (duration(seconds))
                                } @else {
                                    "running"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn run_detail(state: &AppState, ctx: &Ctx, run: &Run) -> Markup {
    let badge = status_badge(run.status);
    let lines = state.store.log_lines(&run.id);
    let live_island = !ctx.face.is_small_screen();
    html! {
        div class="stack" {
            header class="stack" {
                div class="row" {
                    a class="button button-quiet" href="/runs" { "← Runs" }
                    span class=(badge.class) {
                        span class="glyph" aria-hidden="true" { (badge.glyph) }
                        (badge.label)
                    }
                }
                h1 { (run.repository) " · " (run.workflow) }
                p class="lede" { (run.message) }
                dl class="row" {
                    div { dt class="visually-hidden" { "Branch" } dd class="mono faint" { (run.branch) " @ " (run.commit) } }
                    div { dt class="visually-hidden" { "Triggered by" } dd class="faint" { "by " (run.actor) } }
                    @if let Some(runner) = &run.runner {
                        div { dt class="visually-hidden" { "Runner" } dd class="faint" { "on " (runner) } }
                    }
                    @if let Some(seconds) = run.duration_seconds {
                        div { dt class="visually-hidden" { "Duration" } dd class="mono faint" { (duration(seconds)) } }
                    }
                }
            }

            section class="card stack" aria-labelledby="log-heading" {
                div class="row-between" {
                    h2 id="log-heading" { "Log" }
                    div class="row" {
                        @if live_island {
                            p class="tail-status" id="tail-status" data-state="connecting" aria-live="polite" {
                                "Connecting…"
                            }
                            button type="button" id="tail-pause" class="button button-quiet"
                                aria-pressed="false" { "Pause" }
                        } @else {
                            p class="tail-status" { "Reload to see newer output." }
                        }
                    }
                }
                // Server-rendered so the page is readable before, and without, any script. The
                // socket appends to this same list, continuing from `from` in the inline config.
                @let log_label = format!("Build log for {} {}", run.repository, run.workflow);
                ol class="logtail" id="logtail" tabindex="0" role="log" aria-label=(log_label) {
                    @for (index, line) in lines.iter().enumerate() {
                        li {
                            span class="seq" aria-hidden="true" { (index + 1) }
                            span class="line" { (line) }
                        }
                    }
                }
                @if lines.is_empty() {
                    p class="muted" { "Nothing has been written to this log yet." }
                }
                @if !live_island && run.is_live() {
                    p class="faint" { "This run is still going. The live tail is available on a larger screen." }
                }
            }
        }
    }
}

fn runners_page(state: &AppState) -> Markup {
    let runners = state.store.runners();
    html! {
        div class="stack" {
            header class="stack-tight" {
                h1 { "Runners" }
                p class="lede" {
                    "Machines you host. A runner that has not reported in is shown as quiet, not \
                     as idle — those are different problems."
                }
            }
            section class="card" { (runner_table(&runners)) }
        }
    }
}

fn runner_table(runners: &[Runner]) -> Markup {
    html! {
        div class="table-scroll" {
            table {
                caption class="visually-hidden" { "Self-hosted runners" }
                thead { tr {
                    th scope="col" { "State" }
                    th scope="col" { "Runner" }
                    th scope="col" { "Labels" }
                    th scope="col" { "Doing" }
                } }
                tbody {
                    @for runner in runners {
                        tr {
                            td {
                                @if runner.online {
                                    span class="badge badge-ok" {
                                        span class="glyph" aria-hidden="true" { "●" }
                                        "Online"
                                    }
                                } @else {
                                    span class="badge badge-muted" {
                                        span class="glyph" aria-hidden="true" { "○" }
                                        "Quiet"
                                    }
                                }
                            }
                            td {
                                div class="stack-tight" {
                                    strong { (runner.name) }
                                    span class="faint" { (runner.operating_system) " · " (runner.architecture) }
                                }
                            }
                            td class="mono faint" { (runner.labels.join(", ")) }
                            td class="faint" {
                                @if let Some(run) = &runner.current_run {
                                    @let href = format!("/runs/{run}");
                                    a href=(href) { "a run" }
                                } @else {
                                    "idle"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn caches_page() -> Markup {
    html! {
        div class="stack" {
            header class="stack-tight" {
                h1 { "Caches" }
                p class="lede" { "Not wired to the cache service yet." }
            }
            div class="card stack" {
                p {
                    "Cache hit rate and size per repository will land here. It is deliberately \
                     empty rather than showing invented numbers: a dashboard that lies once is \
                     never trusted again."
                }
                p class="muted" {
                    "The API server publishes cache events on the same session protocol the log \
                     tail uses, so this page becomes another subscriber rather than another poller."
                }
            }
        }
    }
}

fn settings_page(state: &AppState, ctx: &Ctx) -> Markup {
    let organization = state.store.organization();
    let org_link = super::layout::surface_url(
        &state.config.apex,
        state.config.cookies_are_secure(),
        "org",
        "/members",
    );
    html! {
        div class="stack" {
            header class="stack-tight" {
                h1 { "Settings" }
                @if let Some(session) = &ctx.session {
                    p class="lede" { "Signed in as " span class="mono" { (session.email) } "." }
                }
            }
            section class="card stack" {
                h2 { "Workspace" }
                dl class="stack-tight" {
                    div class="row-between" {
                        dt class="muted" { "Organization" }
                        dd { (organization.name) }
                    }
                    div class="row-between" {
                        dt class="muted" { "Your role" }
                        dd { (ctx.session.as_ref().map_or("—", |session| session.role().as_str())) }
                    }
                }
                p { a class="button" href=(org_link) { "Manage people and seats" } }
            }
            section class="card stack" {
                h2 { "Live log tail" }
                p class="muted" {
                    "The tail grants the server a fixed amount of credit and tops it up as lines \
                     arrive. Pausing stops granting, which stops the server sending — nothing is \
                     buffered on your behalf, and a gap is always reported rather than hidden."
                }
            }
        }
    }
}

/// `486` → `8m 6s`.
#[must_use]
pub fn duration(seconds: i64) -> String {
    if seconds < 60 {
        return format!("{seconds}s");
    }
    let minutes = seconds / 60;
    let rest = seconds % 60;
    if minutes < 60 {
        return format!("{minutes}m {rest}s");
    }
    format!("{}h {}m", minutes / 60, minutes % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_read_as_english() {
        assert_eq!(duration(0), "0s");
        assert_eq!(duration(31), "31s");
        assert_eq!(duration(60), "1m 0s");
        assert_eq!(duration(486), "8m 6s");
        assert_eq!(duration(3_600), "1h 0m");
        assert_eq!(duration(7_845), "2h 10m");
    }

    #[test]
    fn a_run_id_that_is_not_one_produces_no_inline_script() {
        // The guard is `present::is_run_id`; this asserts the page honours it rather than
        // trusting the router.
        assert!(!crate::present::is_run_id("a\"; alert(1); //"));
        assert!(!crate::present::is_run_id(""));
        assert!(crate::present::is_run_id("01HZY7Q0J8KDPM4V2XN6ABCD"));
    }
}
