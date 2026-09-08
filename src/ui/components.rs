#![forbid(unsafe_code)]

//! Small, composable pieces every surface shares.
//!
//! Each one is a pure `&…` → [`Markup`] function: no state, no I/O, no request.
//! That makes them trivially snapshot-testable and impossible to accidentally
//! couple to a particular host.

use maud::{html, Markup};

use crate::error::WebError;

/// Severity of an [`alert`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tone {
    Neutral,
    Ok,
    Warn,
    Bad,
}

impl Tone {
    #[must_use]
    pub const fn class(self) -> &'static str {
        match self {
            Self::Neutral => "",
            Self::Ok => "ok",
            Self::Warn => "warn",
            Self::Bad => "bad",
        }
    }

    /// Maps a run/job status string onto a tone.
    #[must_use]
    pub fn for_status(status: &str) -> Self {
        match status.trim().to_ascii_lowercase().as_str() {
            "succeeded" | "success" | "passed" | "healthy" | "active" | "verified" => Self::Ok,
            "failed" | "failure" | "cancelled" | "canceled" | "timed_out" | "errored" | "offline" => Self::Bad,
            "queued" | "pending" | "running" | "in_progress" | "draining" | "unverified" => Self::Warn,
            _ => Self::Neutral,
        }
    }
}

/// A section wrapper with the marketing site's numbered eyebrow.
#[must_use]
pub fn section(eyebrow: &str, heading: &str, body: Markup) -> Markup {
    html! {
        section class="section" {
            @if !eyebrow.is_empty() {
                p class="section-number" { (eyebrow) }
            }
            @if !heading.is_empty() {
                h2 { (heading) }
            }
            (body)
        }
    }
}

/// A bordered card with an optional mono label.
#[must_use]
pub fn card(label: &str, body: Markup) -> Markup {
    html! {
        article class="card" {
            @if !label.is_empty() {
                span class="label" { (label) }
            }
            (body)
        }
    }
}

/// A single status pill.
#[must_use]
pub fn badge(text: &str, tone: Tone) -> Markup {
    let class = format!("badge {}", tone.class());
    html! { span class=(class.trim()) { (text) } }
}

/// A status pill derived from a status string.
#[must_use]
pub fn status_badge(status: &str) -> Markup {
    badge(status, Tone::for_status(status))
}

/// A labelled number, used on dashboards.
#[must_use]
pub fn stat(label: &str, value: &str, note: &str) -> Markup {
    html! {
        article class="card" {
            span class="label" { (label) }
            h3 { (value) }
            @if !note.is_empty() {
                p { (note) }
            }
        }
    }
}

/// A prominent message block.
#[must_use]
pub fn alert(tone: Tone, heading: &str, detail: &str) -> Markup {
    let class = match tone {
        Tone::Bad => "alert error",
        Tone::Ok => "alert ok",
        _ => "alert",
    };
    html! {
        div class=(class) role="status" {
            h3 { (heading) }
            p { (detail) }
        }
    }
}

/// The body of an error response. Rendered bare so it works both as a page
/// section and as an htmx swap target.
#[must_use]
pub fn error_block(error: &WebError) -> Markup {
    html! {
        div class="alert error" role="alert" data-error=(error.code()) {
            h3 { (error.headline()) }
            p { (error.detail()) }
        }
    }
}

/// Placeholder for an empty collection. Always says what to do next.
#[must_use]
pub fn empty_state(message: &str, action_label: &str, action_href: &str) -> Markup {
    html! {
        div class="empty" {
            p { (message) }
            @if !action_label.is_empty() {
                a class="button secondary" href=(action_href) { (action_label) }
            }
        }
    }
}

/// One step of the org onboarding wizard.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StepState {
    Done,
    Current,
    Upcoming,
}

impl StepState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Current => "current",
            Self::Upcoming => "upcoming",
        }
    }
}

/// The wizard progress rail. `steps` is `(index, label, state)`.
#[must_use]
pub fn step_indicator(steps: &[(usize, &str, StepState)]) -> Markup {
    html! {
        ol class="steps" {
            @for (index, label, state) in steps {
                li data-state=(state.as_str()) {
                    span class="index" { (format!("{index:02}")) }
                    span { (label) }
                }
            }
        }
    }
}

/// The live log surface for a run.
///
/// `hx-ext="ws"` plus `ws-connect` is the streaming path; the `hx-get` polling
/// attributes on the same element are the fallback when the WebSocket cannot be
/// established (corporate proxies, the `ws-relay` feature compiled out). One of
/// the two always produces output, and both write into the same container.
#[must_use]
pub fn log_stream(run_id: &str, initial: &[String], live: bool) -> Markup {
    let ws_path = format!("/ws?run_id={run_id}");
    let poll_path = format!("/runs/{run_id}/log");
    html! {
        div class="row" {
            span class="live-dot" aria-hidden="true" {}
            span class="text-link" { @if live { "Streaming" } @else { "Polling every 3s" } }
            span class="htmx-indicator text-link" { "…" }
        }
        @if live {
            div id="log-stream" class="logs" role="log" aria-live="polite"
                hx-ext="ws" ws-connect=(ws_path) {
                (log_lines(initial))
            }
        } @else {
            div id="log-stream" class="logs" role="log" aria-live="polite"
                hx-get=(poll_path) hx-trigger="load, every 3s" hx-swap="innerHTML" {
                (log_lines(initial))
            }
        }
    }
}

/// Log lines as an htmx swap payload.
#[must_use]
pub fn log_lines(lines: &[String]) -> Markup {
    html! {
        @for line in lines {
            span class="line" { (line) }
        }
        @if lines.is_empty() {
            span class="line" { "No output yet." }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_map_to_the_expected_tone() {
        assert_eq!(Tone::for_status("succeeded"), Tone::Ok);
        assert_eq!(Tone::for_status("FAILED"), Tone::Bad);
        assert_eq!(Tone::for_status("queued"), Tone::Warn);
        assert_eq!(Tone::for_status("something-new"), Tone::Neutral);
    }

    #[test]
    fn a_neutral_badge_has_no_trailing_class_whitespace() {
        assert_eq!(
            badge("queued", Tone::Neutral).into_string(),
            r#"<span class="badge">queued</span>"#
        );
        assert_eq!(
            status_badge("succeeded").into_string(),
            r#"<span class="badge ok">succeeded</span>"#
        );
    }

    #[test]
    fn user_supplied_text_is_escaped_everywhere() {
        let hostile = "<img src=x onerror=alert(1)>";
        for markup in [
            badge(hostile, Tone::Ok).into_string(),
            stat("l", hostile, hostile).into_string(),
            alert(Tone::Bad, hostile, hostile).into_string(),
            empty_state(hostile, "", "").into_string(),
            log_lines(&[hostile.to_owned()]).into_string(),
        ] {
            assert!(!markup.contains("<img"), "unescaped markup: {markup}");
            assert!(markup.contains("&lt;img"));
        }
    }

    #[test]
    fn the_error_block_carries_a_machine_code_and_no_internals() {
        let html = error_block(&WebError::CsrfRejected).into_string();
        assert!(html.contains(r#"data-error="csrf_rejected""#));
        assert!(html.contains("That form expired"));
        assert!(html.contains(r#"role="alert""#));
    }

    #[test]
    fn the_log_surface_streams_when_live_and_polls_otherwise() {
        let live = log_stream("run_1", &[], true).into_string();
        assert!(live.contains(r#"ws-connect="/ws?run_id=run_1""#));
        assert!(live.contains(r#"hx-ext="ws""#));
        assert!(!live.contains("hx-trigger"));

        let polled = log_stream("run_1", &[], false).into_string();
        assert!(polled.contains(r#"hx-get="/runs/run_1/log""#));
        assert!(polled.contains(r#"hx-trigger="load, every 3s""#));
        assert!(!polled.contains("ws-connect"));
    }

    #[test]
    fn an_empty_log_says_so_rather_than_rendering_nothing() {
        assert!(log_lines(&[]).into_string().contains("No output yet."));
    }

    #[test]
    fn step_states_are_exposed_for_styling_and_tests() {
        let html = step_indicator(&[
            (1, "Create organization", StepState::Done),
            (2, "Verify domain", StepState::Current),
        ])
        .into_string();
        assert!(html.contains(r#"data-state="done""#));
        assert!(html.contains(r#"data-state="current""#));
        assert!(html.contains("<span class=\"index\">01</span>"));
    }
}
