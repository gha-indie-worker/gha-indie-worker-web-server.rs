#![forbid(unsafe_code)]

//! Database row projections.
//!
//! These mirror, one field per selected column, the entities that
//! `gha-indie-worker-orm-core` generates from the JSON-Schema/TypeSpec
//! authorities in `gha-indie-worker-interfaces`. That crate is private and does
//! not exist yet; when it lands, this module becomes a set of `From<orm_core::…>`
//! conversions and the column lists in `super::db::sql` are deleted in favour
//! of the generated entities. Nothing outside `super::db` constructs a row, so
//! that swap touches two files.
//!
//! Rows are separate from [`super::models`] on purpose: a column rename must not
//! be able to reach a template.

use super::models::{AuditEntry, OrgMember, PlanSummary, RunJob, RunSummary, SeatUsage, WorkerSummary};

/// `runs`
#[derive(Clone, Debug, Default)]
pub struct RunRow {
    pub id: String,
    pub repository: String,
    pub revision: String,
    pub workflow_path: String,
    pub status: String,
    pub created_at: String,
    pub duration_seconds: Option<i64>,
    pub profile: Option<String>,
}

impl From<RunRow> for RunSummary {
    fn from(row: RunRow) -> Self {
        Self {
            id: row.id,
            repository: row.repository,
            revision: row.revision,
            workflow_path: row.workflow_path,
            status: row.status,
            created_at: row.created_at,
            duration: row.duration_seconds.map(format_duration),
            profile: row.profile,
        }
    }
}

/// `run_jobs`
#[derive(Clone, Debug, Default)]
pub struct RunJobRow {
    pub id: String,
    pub name: String,
    pub status: String,
    pub profile: Option<String>,
    pub duration_seconds: Option<i64>,
    pub lane: Option<String>,
}

impl From<RunJobRow> for RunJob {
    fn from(row: RunJobRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            status: row.status,
            profile: row.profile,
            duration: row.duration_seconds.map(format_duration),
            lane: row.lane,
        }
    }
}

/// `workers`
#[derive(Clone, Debug, Default)]
pub struct WorkerRow {
    pub id: String,
    pub name: String,
    pub os: String,
    pub arch: String,
    pub status: String,
    pub last_seen_at: Option<String>,
    pub profiles: Option<String>,
    pub certified: bool,
}

impl From<WorkerRow> for WorkerSummary {
    fn from(row: WorkerRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            os: row.os,
            arch: row.arch,
            status: row.status,
            last_seen_at: row.last_seen_at,
            profiles: split_list(row.profiles.as_deref()),
            certified: row.certified,
        }
    }
}

/// `build_profiles`
#[derive(Clone, Debug, Default)]
pub struct PlanRow {
    pub name: String,
    pub description: String,
    pub evidence: Option<String>,
    pub enabled: bool,
}

impl From<PlanRow> for PlanSummary {
    fn from(row: PlanRow) -> Self {
        Self {
            name: row.name,
            description: row.description,
            evidence: split_list(row.evidence.as_deref()),
            enabled: row.enabled,
        }
    }
}

/// `org_members`
#[derive(Clone, Debug, Default)]
pub struct OrgMemberRow {
    pub id: String,
    pub email: String,
    pub role: String,
    pub status: String,
    pub invited_at: Option<String>,
    pub last_active_at: Option<String>,
}

impl From<OrgMemberRow> for OrgMember {
    fn from(row: OrgMemberRow) -> Self {
        Self {
            id: row.id,
            email: row.email,
            role: row.role,
            status: row.status,
            invited_at: row.invited_at,
            last_active_at: row.last_active_at,
        }
    }
}

/// `org_seats`
#[derive(Clone, Copy, Debug, Default)]
pub struct SeatRow {
    pub allocated: i64,
    pub used: i64,
    pub pending_invites: i64,
}

impl From<SeatRow> for SeatUsage {
    fn from(row: SeatRow) -> Self {
        Self {
            allocated: clamp_u32(row.allocated),
            used: clamp_u32(row.used),
            pending_invites: clamp_u32(row.pending_invites),
        }
    }
}

/// `audit_log`
#[derive(Clone, Debug, Default)]
pub struct AuditRow {
    pub id: String,
    pub at: String,
    pub actor: String,
    pub action: String,
    pub target: Option<String>,
    pub result: Option<String>,
}

impl From<AuditRow> for AuditEntry {
    fn from(row: AuditRow) -> Self {
        Self {
            id: row.id,
            at: row.at,
            actor: row.actor,
            action: row.action,
            target: row.target,
            result: row.result.unwrap_or_else(|| "ok".to_owned()),
        }
    }
}

/// `1h 02m 03s`, `2m 03s`, `12s`. Never a bare float.
#[must_use]
pub fn format_duration(seconds: i64) -> String {
    let seconds = seconds.max(0);
    let (hours, minutes, secs) = (seconds / 3_600, (seconds % 3_600) / 60, seconds % 60);
    if hours > 0 {
        format!("{hours}h {minutes:02}m {secs:02}s")
    } else if minutes > 0 {
        format!("{minutes}m {secs:02}s")
    } else {
        format!("{secs}s")
    }
}

/// Postgres text arrays arrive as a comma-joined string from `array_to_string`.
#[must_use]
pub fn split_list(value: Option<&str>) -> Vec<String> {
    value
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn clamp_u32(value: i64) -> u32 {
    u32::try_from(value.max(0)).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_read_as_time_not_as_a_number() {
        assert_eq!(format_duration(0), "0s");
        assert_eq!(format_duration(12), "12s");
        assert_eq!(format_duration(123), "2m 03s");
        assert_eq!(format_duration(3_723), "1h 02m 03s");
        assert_eq!(format_duration(-5), "0s");
    }

    #[test]
    fn lists_survive_being_empty_or_absent() {
        assert!(split_list(None).is_empty());
        assert!(split_list(Some("")).is_empty());
        assert_eq!(
            split_list(Some("a, b ,,c")),
            vec!["a".to_owned(), "b".to_owned(), "c".to_owned()]
        );
    }

    #[test]
    fn a_run_row_becomes_a_summary_with_a_readable_duration() {
        let summary: RunSummary = RunRow {
            id: "run_1".into(),
            repository: "gha-indie-worker/x".into(),
            revision: "0".repeat(40),
            workflow_path: ".github/workflows/ci.yml".into(),
            status: "succeeded".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            duration_seconds: Some(75),
            profile: Some("rust-verify".into()),
        }
        .into();
        assert_eq!(summary.duration.as_deref(), Some("1m 15s"));
        assert_eq!(summary.short_revision(), "0000000");
    }

    #[test]
    fn negative_or_oversized_seat_counts_cannot_wrap() {
        let usage: SeatUsage = SeatRow {
            allocated: -3,
            used: i64::MAX,
            pending_invites: 0,
        }
        .into();
        assert_eq!(usage.allocated, 0);
        assert_eq!(usage.used, u32::MAX);
        assert_eq!(usage.available(), 0);
    }

    #[test]
    fn an_audit_row_without_a_result_defaults_to_ok() {
        let entry: AuditEntry = AuditRow {
            result: None,
            ..AuditRow::default()
        }
        .into();
        assert_eq!(entry.result, "ok");
    }
}
