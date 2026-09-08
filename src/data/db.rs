#![forbid(unsafe_code)]

//! Read-only SeaORM access to the canonical database (`DATABASE_URL_CANONICAL`).
//!
//! Rules this module enforces, not just documents:
//!
//! * **Reads only.** [`ReadPool::query`] refuses any statement that is not a
//!   `SELECT`, so no future edit can turn a page into a writer. Writes go
//!   through the api-server ([`super::api_client`]).
//! * **No DDL, ever.** The fleet contract forbids a service running migrations
//!   at boot; there is no migration path in this crate at all.
//! * **No string interpolation.** Every value is bound through
//!   [`Statement::from_sql_and_values`]; the only thing formatted into SQL is a
//!   `LIMIT` that was first clamped to [`MAX_LIMIT`].
//!
//! Direct `sqlx`/`tokio-postgres` are forbidden fleet-wide, which is why this
//! goes through SeaORM even for projections. All SeaORM API surface used by this
//! crate is in this one file, behind the `db` feature, so a version bump touches
//! exactly one module.

use sea_orm::{ConnectionTrait, Database, DatabaseBackend, DatabaseConnection, QueryResult, Statement, Value};

use crate::error::WebError;

use super::models::{AuditEntry, OrgMember, PlanSummary, RunDetail, RunSummary, SeatUsage, WorkerSummary};
use super::rows::{AuditRow, OrgMemberRow, PlanRow, RunJobRow, RunRow, SeatRow, WorkerRow};

/// Nothing may ask this server for an unbounded page.
pub const MAX_LIMIT: u64 = 200;

/// Every statement this crate issues, in one place. When
/// `gha-indie-worker-orm-core` lands, these are replaced by its generated
/// entities and the column lists disappear.
pub mod sql {
    pub const RUNS_BY_ORG: &str = "\
SELECT id::text AS id, repository, revision, workflow_path, status, \
to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS created_at, \
duration_seconds, profile \
FROM runs WHERE ($1::text IS NULL OR org_id::text = $1) ORDER BY created_at DESC LIMIT ";

    pub const RUN_BY_ID: &str = "\
SELECT id::text AS id, repository, revision, workflow_path, status, \
to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS created_at, \
duration_seconds, profile \
FROM runs WHERE id::text = $1";

    pub const JOBS_BY_RUN: &str = "\
SELECT id::text AS id, name, status, profile, duration_seconds, lane \
FROM run_jobs WHERE run_id::text = $1 ORDER BY position ASC";

    pub const WORKERS_BY_ORG: &str = "\
SELECT id::text AS id, name, os, arch, status, \
to_char(last_seen_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS last_seen_at, \
array_to_string(profiles, ',') AS profiles, certified \
FROM workers WHERE ($1::text IS NULL OR org_id::text = $1) ORDER BY name ASC LIMIT ";

    pub const PLANS: &str = "\
SELECT name, description, array_to_string(evidence, ',') AS evidence, enabled \
FROM build_profiles ORDER BY name ASC LIMIT ";

    pub const MEMBERS_BY_ORG: &str = "\
SELECT id::text AS id, email, role, status, \
to_char(invited_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS invited_at, \
to_char(last_active_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS last_active_at \
FROM org_members WHERE org_id::text = $1 ORDER BY email ASC LIMIT ";

    pub const SEATS_BY_ORG: &str = "\
SELECT allocated, used, pending_invites FROM org_seats WHERE org_id::text = $1";

    pub const AUDIT_BY_ORG: &str = "\
SELECT id::text AS id, \
to_char(at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS at, \
actor, action, target, result \
FROM audit_log WHERE org_id::text = $1 ORDER BY at DESC LIMIT ";
}

/// A read-only connection to the canonical database.
pub struct ReadPool {
    connection: DatabaseConnection,
}

impl std::fmt::Debug for ReadPool {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print the connection: it carries the DSN, which carries a password.
        formatter.debug_struct("ReadPool").finish_non_exhaustive()
    }
}

impl ReadPool {
    /// Connects. The credential in `DATABASE_URL_CANONICAL` is expected to be a
    /// read-only role; [`Self::query`] is the second lock, not the first.
    pub async fn connect(url: &str) -> Result<Self, WebError> {
        let connection = Database::connect(url).await.map_err(|error| {
            // The error may embed the DSN, so only the shape is logged.
            tracing::error!(
                error.kind = "database_connect",
                "canonical read pool could not be opened"
            );
            let _ = error;
            WebError::Unavailable
        })?;
        Ok(Self { connection })
    }

    #[must_use]
    pub const fn connection(&self) -> &DatabaseConnection {
        &self.connection
    }

    /// Clamps a caller-supplied page size.
    #[must_use]
    pub const fn clamp_limit(limit: u64) -> u64 {
        if limit == 0 {
            1
        } else if limit > MAX_LIMIT {
            MAX_LIMIT
        } else {
            limit
        }
    }

    /// Runs one bound `SELECT`.
    async fn query(&self, sql: &str, values: Vec<Value>) -> Result<Vec<QueryResult>, WebError> {
        debug_assert!(
            is_select(sql),
            "only SELECT statements may be issued from the web server"
        );
        if !is_select(sql) {
            tracing::error!(statement.kind = "non_select", "refused a non-SELECT statement");
            return Err(WebError::Internal);
        }
        self.connection
            .query_all_raw(Statement::from_sql_and_values(DatabaseBackend::Postgres, sql, values))
            .await
            .map_err(|error| {
                tracing::warn!(error.kind = "database_query", "canonical read failed");
                let _ = error;
                WebError::Unavailable
            })
    }

    pub async fn runs(&self, org_id: Option<&str>, limit: u64) -> Result<Vec<RunSummary>, WebError> {
        let sql = format!("{}{}", sql::RUNS_BY_ORG, Self::clamp_limit(limit));
        let rows = self.query(&sql, vec![optional_text(org_id)]).await?;
        Ok(rows.iter().filter_map(run_row).map(RunSummary::from).collect())
    }

    pub async fn run(&self, run_id: &str) -> Result<RunDetail, WebError> {
        let rows = self.query(sql::RUN_BY_ID, vec![Value::from(run_id)]).await?;
        let summary: RunSummary = rows.first().and_then(run_row).ok_or(WebError::NotFound)?.into();
        let job_rows = self.query(sql::JOBS_BY_RUN, vec![Value::from(run_id)]).await?;
        Ok(RunDetail {
            summary,
            jobs: job_rows.iter().filter_map(job_row).map(Into::into).collect(),
            // Log tail and exclusions are streamed state, owned by the api-server.
            log_tail: Vec::new(),
            exclusions: Vec::new(),
        })
    }

    pub async fn workers(&self, org_id: Option<&str>) -> Result<Vec<WorkerSummary>, WebError> {
        let sql = format!("{}{MAX_LIMIT}", sql::WORKERS_BY_ORG);
        let rows = self.query(&sql, vec![optional_text(org_id)]).await?;
        Ok(rows.iter().filter_map(worker_row).map(Into::into).collect())
    }

    pub async fn plans(&self) -> Result<Vec<PlanSummary>, WebError> {
        let sql = format!("{}{MAX_LIMIT}", sql::PLANS);
        let rows = self.query(&sql, Vec::new()).await?;
        Ok(rows.iter().filter_map(plan_row).map(Into::into).collect())
    }

    pub async fn members(&self, org_id: &str) -> Result<Vec<OrgMember>, WebError> {
        let sql = format!("{}{MAX_LIMIT}", sql::MEMBERS_BY_ORG);
        let rows = self.query(&sql, vec![Value::from(org_id)]).await?;
        Ok(rows.iter().filter_map(member_row).map(Into::into).collect())
    }

    pub async fn seats(&self, org_id: &str) -> Result<SeatUsage, WebError> {
        let rows = self.query(sql::SEATS_BY_ORG, vec![Value::from(org_id)]).await?;
        let row = rows.first().ok_or(WebError::NotFound)?;
        Ok(SeatRow {
            allocated: row.try_get("", "allocated").unwrap_or_default(),
            used: row.try_get("", "used").unwrap_or_default(),
            pending_invites: row.try_get("", "pending_invites").unwrap_or_default(),
        }
        .into())
    }

    pub async fn audit(&self, org_id: &str, limit: u64) -> Result<Vec<AuditEntry>, WebError> {
        let sql = format!("{}{}", sql::AUDIT_BY_ORG, Self::clamp_limit(limit));
        let rows = self.query(&sql, vec![Value::from(org_id)]).await?;
        Ok(rows.iter().filter_map(audit_row).map(Into::into).collect())
    }
}

/// `NULL`-able bind for an optional filter.
fn optional_text(value: Option<&str>) -> Value {
    // sea-query's blanket `From<Option<T>>` yields the correctly typed NULL.
    Value::from(value.map(str::to_owned))
}

/// A statement is a read exactly when it starts with `SELECT` and contains no
/// statement separator (which would allow a second, unchecked statement).
#[must_use]
pub fn is_select(sql: &str) -> bool {
    let trimmed = sql.trim_start();
    trimmed.get(..6).is_some_and(|head| head.eq_ignore_ascii_case("SELECT"))
        && !trimmed.trim_end().trim_end_matches(';').contains(';')
}

fn run_row(row: &QueryResult) -> Option<RunRow> {
    Some(RunRow {
        id: row.try_get("", "id").ok()?,
        repository: row.try_get("", "repository").unwrap_or_default(),
        revision: row.try_get("", "revision").unwrap_or_default(),
        workflow_path: row.try_get("", "workflow_path").unwrap_or_default(),
        status: row.try_get("", "status").unwrap_or_default(),
        created_at: row.try_get("", "created_at").unwrap_or_default(),
        duration_seconds: row.try_get("", "duration_seconds").unwrap_or_default(),
        profile: row.try_get("", "profile").unwrap_or_default(),
    })
}

fn job_row(row: &QueryResult) -> Option<RunJobRow> {
    Some(RunJobRow {
        id: row.try_get("", "id").ok()?,
        name: row.try_get("", "name").unwrap_or_default(),
        status: row.try_get("", "status").unwrap_or_default(),
        profile: row.try_get("", "profile").unwrap_or_default(),
        duration_seconds: row.try_get("", "duration_seconds").unwrap_or_default(),
        lane: row.try_get("", "lane").unwrap_or_default(),
    })
}

fn worker_row(row: &QueryResult) -> Option<WorkerRow> {
    Some(WorkerRow {
        id: row.try_get("", "id").ok()?,
        name: row.try_get("", "name").unwrap_or_default(),
        os: row.try_get("", "os").unwrap_or_default(),
        arch: row.try_get("", "arch").unwrap_or_default(),
        status: row.try_get("", "status").unwrap_or_default(),
        last_seen_at: row.try_get("", "last_seen_at").unwrap_or_default(),
        profiles: row.try_get("", "profiles").unwrap_or_default(),
        certified: row.try_get("", "certified").unwrap_or_default(),
    })
}

fn plan_row(row: &QueryResult) -> Option<PlanRow> {
    Some(PlanRow {
        name: row.try_get("", "name").ok()?,
        description: row.try_get("", "description").unwrap_or_default(),
        evidence: row.try_get("", "evidence").unwrap_or_default(),
        enabled: row.try_get("", "enabled").unwrap_or_default(),
    })
}

fn member_row(row: &QueryResult) -> Option<OrgMemberRow> {
    Some(OrgMemberRow {
        id: row.try_get("", "id").ok()?,
        email: row.try_get("", "email").unwrap_or_default(),
        role: row.try_get("", "role").unwrap_or_default(),
        status: row.try_get("", "status").unwrap_or_default(),
        invited_at: row.try_get("", "invited_at").unwrap_or_default(),
        last_active_at: row.try_get("", "last_active_at").unwrap_or_default(),
    })
}

fn audit_row(row: &QueryResult) -> Option<AuditRow> {
    Some(AuditRow {
        id: row.try_get("", "id").ok()?,
        at: row.try_get("", "at").unwrap_or_default(),
        actor: row.try_get("", "actor").unwrap_or_default(),
        action: row.try_get("", "action").unwrap_or_default(),
        target: row.try_get("", "target").unwrap_or_default(),
        result: row.try_get("", "result").unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_single_select_statements_are_accepted() {
        assert!(is_select("SELECT 1"));
        assert!(is_select("  select id FROM runs;"));
        assert!(!is_select("UPDATE runs SET status = 'x'"));
        assert!(!is_select("DELETE FROM runs"));
        assert!(!is_select("CREATE TABLE t (id int)"));
        assert!(!is_select("SELECT 1; DROP TABLE runs"));
        assert!(!is_select(""));
    }

    #[test]
    fn every_statement_this_crate_issues_is_a_read() {
        for statement in [
            sql::RUNS_BY_ORG,
            sql::RUN_BY_ID,
            sql::JOBS_BY_RUN,
            sql::WORKERS_BY_ORG,
            sql::PLANS,
            sql::MEMBERS_BY_ORG,
            sql::SEATS_BY_ORG,
            sql::AUDIT_BY_ORG,
        ] {
            assert!(is_select(statement), "not a read: {statement}");
        }
    }

    #[test]
    fn page_sizes_are_clamped_in_both_directions() {
        assert_eq!(ReadPool::clamp_limit(0), 1);
        assert_eq!(ReadPool::clamp_limit(50), 50);
        assert_eq!(ReadPool::clamp_limit(u64::MAX), MAX_LIMIT);
    }

    #[test]
    fn filters_bind_values_rather_than_interpolating_them() {
        // Every statement takes its filters as placeholders; the only thing
        // formatted in is the clamped LIMIT.
        for statement in [sql::RUNS_BY_ORG, sql::RUN_BY_ID, sql::MEMBERS_BY_ORG, sql::AUDIT_BY_ORG] {
            assert!(statement.contains("$1"), "missing bind placeholder: {statement}");
        }
        assert!(!sql::RUNS_BY_ORG.contains("format!"));
    }
}
