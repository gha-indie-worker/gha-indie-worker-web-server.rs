#![forbid(unsafe_code)]

//! View models.
//!
//! These are the shapes pages render. They are deliberately *not* the wire
//! contract and *not* the database row: both of those live elsewhere (the
//! JSON-Schema/TypeSpec authorities in `gha-indie-worker-interfaces`, and
//! [`super::rows`]). Keeping a third, page-shaped type means a column rename or
//! a contract addition changes one mapping function, not forty templates.
//!
//! Every timestamp is already a display string. Formatting happens at the edge
//! that knows the reader's context, not in a template.

use serde::{Deserialize, Serialize};

/// A run in a list.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSummary {
    pub id: String,
    pub repository: String,
    /// Immutable 40-hex commit SHA. The independent lane never runs a branch.
    pub revision: String,
    pub workflow_path: String,
    pub status: String,
    pub created_at: String,
    #[serde(default)]
    pub duration: Option<String>,
    #[serde(default)]
    pub profile: Option<String>,
}

impl RunSummary {
    /// The first 7 characters of the SHA, as every git UI shows it.
    #[must_use]
    pub fn short_revision(&self) -> &str {
        let end = self
            .revision
            .char_indices()
            .nth(7)
            .map_or(self.revision.len(), |(index, _)| index);
        &self.revision[..end]
    }

    #[must_use]
    pub fn href(&self) -> String {
        format!("/runs/{}", self.id)
    }

    /// A run still producing output should stream rather than poll.
    #[must_use]
    pub fn is_live(&self) -> bool {
        matches!(self.status.as_str(), "queued" | "pending" | "running" | "in_progress")
    }
}

/// One job inside a run.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunJob {
    pub id: String,
    pub name: String,
    pub status: String,
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub duration: Option<String>,
    /// ARC classification when the job did not take the independent lane.
    #[serde(default)]
    pub lane: Option<String>,
}

/// A run detail page.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunDetail {
    #[serde(flatten)]
    pub summary: RunSummary,
    #[serde(default)]
    pub jobs: Vec<RunJob>,
    #[serde(default)]
    pub log_tail: Vec<String>,
    /// Explicitly reported unsupported behaviour, never silently approximated.
    #[serde(default)]
    pub exclusions: Vec<String>,
}

/// A registered worker.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerSummary {
    pub id: String,
    pub name: String,
    pub os: String,
    pub arch: String,
    pub status: String,
    #[serde(default)]
    pub last_seen_at: Option<String>,
    #[serde(default)]
    pub profiles: Vec<String>,
    /// Whether the runner's declared contract was verified from outside.
    #[serde(default)]
    pub certified: bool,
}

/// A fixed build profile.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanSummary {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub evidence: Vec<String>,
    #[serde(default)]
    pub enabled: bool,
}

/// An organization.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrgSummary {
    pub id: String,
    pub name: String,
    pub slug: String,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub domain_verified: bool,
    #[serde(default)]
    pub billing_url: Option<String>,
}

/// A member of an organization.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrgMember {
    pub id: String,
    pub email: String,
    pub role: String,
    pub status: String,
    #[serde(default)]
    pub invited_at: Option<String>,
    #[serde(default)]
    pub last_active_at: Option<String>,
}

/// Seat allocation for an organization.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeatUsage {
    pub allocated: u32,
    pub used: u32,
    pub pending_invites: u32,
}

impl SeatUsage {
    #[must_use]
    pub const fn available(self) -> u32 {
        self.allocated
            .saturating_sub(self.used)
            .saturating_sub(self.pending_invites)
    }

    #[must_use]
    pub const fn is_exhausted(self) -> bool {
        self.available() == 0
    }
}

/// One audit-log entry.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEntry {
    pub id: String,
    pub at: String,
    pub actor: String,
    pub action: String,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub result: String,
}

/// A personal API token. The secret is shown once, by the api-server, on create.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenSummary {
    pub id: String,
    pub name: String,
    pub created_at: String,
    #[serde(default)]
    pub last_used_at: Option<String>,
    #[serde(default)]
    pub scopes: Vec<String>,
    /// Never the token itself — a display prefix such as `giw_live_9f3a…`.
    #[serde(default)]
    pub prefix: String,
}

/// The result of one invite in a batch.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteOutcome {
    pub email: String,
    pub accepted: bool,
    #[serde(default)]
    pub reason: Option<String>,
}

impl InviteOutcome {
    #[must_use]
    pub fn accepted(email: impl Into<String>) -> Self {
        Self {
            email: email.into(),
            accepted: true,
            reason: None,
        }
    }

    #[must_use]
    pub fn rejected(email: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            email: email.into(),
            accepted: false,
            reason: Some(reason.into()),
        }
    }
}

/// Parses the invite box: one address per line, or a pasted CSV where the first
/// field is the address and an optional second field is the role.
///
/// Returns `(email, role)` pairs. Anything that cannot be an address is reported
/// rather than dropped, so a typo in a 200-line paste is visible.
#[must_use]
pub fn parse_invite_list(raw: &str, default_role: &str) -> (Vec<(String, String)>, Vec<InviteOutcome>) {
    let mut accepted = Vec::new();
    let mut rejected = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for line in raw.lines() {
        let line = line.trim().trim_end_matches(',');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split(&[',', ';', '\t'][..]).map(str::trim);
        let email = fields.next().unwrap_or_default().trim_matches('"').to_ascii_lowercase();
        let role = fields
            .next()
            .filter(|value| !value.is_empty())
            .unwrap_or(default_role)
            .to_owned();
        // A header row from a spreadsheet export is skipped, not reported.
        if email == "email" || email == "e-mail" {
            continue;
        }
        if !is_plausible_email(&email) {
            rejected.push(InviteOutcome::rejected(email, "not an email address"));
            continue;
        }
        if seen.iter().any(|existing| existing == &email) {
            rejected.push(InviteOutcome::rejected(email, "listed more than once"));
            continue;
        }
        seen.push(email.clone());
        accepted.push((email, role));
    }
    (accepted, rejected)
}

/// A deliberately conservative check: exactly one `@`, non-empty on both sides,
/// a dot in the domain, no whitespace. Delivery is the api-server's problem.
#[must_use]
pub fn is_plausible_email(value: &str) -> bool {
    let Some((local, domain)) = value.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && !domain.is_empty()
        && !domain.contains('@')
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !value.chars().any(char::is_whitespace)
        && value.len() <= 254
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_revision_is_seven_characters_and_never_panics() {
        let mut run = RunSummary {
            revision: "0123456789abcdef".into(),
            ..RunSummary::default()
        };
        assert_eq!(run.short_revision(), "0123456");
        run.revision = "abc".into();
        assert_eq!(run.short_revision(), "abc");
        run.revision = String::new();
        assert_eq!(run.short_revision(), "");
    }

    #[test]
    fn live_runs_are_the_ones_still_producing_output() {
        let mut run = RunSummary {
            status: "running".into(),
            ..RunSummary::default()
        };
        assert!(run.is_live());
        run.status = "succeeded".into();
        assert!(!run.is_live());
    }

    #[test]
    fn seat_arithmetic_never_underflows() {
        let usage = SeatUsage {
            allocated: 5,
            used: 4,
            pending_invites: 3,
        };
        assert_eq!(usage.available(), 0);
        assert!(usage.is_exhausted());
        assert_eq!(
            SeatUsage {
                allocated: 10,
                used: 2,
                pending_invites: 1
            }
            .available(),
            7
        );
    }

    #[test]
    fn plausible_emails_are_accepted_and_obvious_junk_is_not() {
        assert!(is_plausible_email("a@example.com"));
        assert!(!is_plausible_email("a@example"));
        assert!(!is_plausible_email("@example.com"));
        assert!(!is_plausible_email("a@@example.com"));
        assert!(!is_plausible_email("a b@example.com"));
        assert!(!is_plausible_email("no-at-sign"));
    }

    #[test]
    fn one_address_per_line_is_the_simple_case() {
        let (accepted, rejected) = parse_invite_list("a@example.com\nb@example.com\n", "member");
        assert_eq!(
            accepted,
            vec![
                ("a@example.com".to_owned(), "member".to_owned()),
                ("b@example.com".to_owned(), "member".to_owned()),
            ]
        );
        assert!(rejected.is_empty());
    }

    #[test]
    fn a_pasted_csv_carries_a_per_row_role() {
        let (accepted, rejected) = parse_invite_list("email,role\nA@Example.com, owner\nb@example.com\n", "member");
        assert_eq!(accepted[0], ("a@example.com".to_owned(), "owner".to_owned()));
        assert_eq!(accepted[1], ("b@example.com".to_owned(), "member".to_owned()));
        assert!(rejected.is_empty(), "the header row must be skipped, not reported");
    }

    #[test]
    fn duplicates_and_typos_are_reported_rather_than_dropped() {
        let (accepted, rejected) = parse_invite_list("a@example.com\na@example.com\nnot-an-email\n", "member");
        assert_eq!(accepted.len(), 1);
        assert_eq!(rejected.len(), 2);
        assert_eq!(rejected[0].reason.as_deref(), Some("listed more than once"));
        assert_eq!(rejected[1].reason.as_deref(), Some("not an email address"));
        assert!(rejected.iter().all(|outcome| !outcome.accepted));
    }

    #[test]
    fn blank_lines_and_comments_are_ignored() {
        let (accepted, rejected) = parse_invite_list("\n\n# a comment\n  \na@example.com\n", "member");
        assert_eq!(accepted.len(), 1);
        assert!(rejected.is_empty());
    }

    #[test]
    fn a_run_detail_deserializes_a_flattened_summary() {
        let json = serde_json::json!({
            "id": "run_1", "repository": "gha-indie-worker/x", "revision": "0".repeat(40),
            "workflowPath": ".github/workflows/ci.yml", "status": "succeeded",
            "createdAt": "2026-01-01T00:00:00Z", "jobs": [], "logTail": ["line"]
        });
        let detail: RunDetail = serde_json::from_value(json).expect("deserializes");
        assert_eq!(detail.summary.id, "run_1");
        assert_eq!(detail.log_tail, vec!["line".to_owned()]);
    }
}
