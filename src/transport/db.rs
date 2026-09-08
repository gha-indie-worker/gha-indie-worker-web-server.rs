#![forbid(unsafe_code)]

//! The direct read-only database avenue.
//!
//! The connection and the statements live in [`crate::data::db`] (feature `db`).
//! What lives here is the avenue's *policy*: what this server is allowed to ask
//! the canonical database for, and what it must not.

use crate::persistence::ReadOnlyProjection;

/// Tables this server may read. Anything absent is answered by the api-server.
///
/// Keeping the list here — rather than implicitly in whichever SQL happens to
/// exist — means widening the web server's reach is a visible diff.
pub const READABLE_TABLES: &[&str] = &[
    "runs",
    "run_jobs",
    "workers",
    "build_profiles",
    "org_members",
    "org_seats",
    "audit_log",
];

/// Tables this server must never read, even though its role might allow it.
/// Token material and auth state belong to the api-server and shared-auth.
pub const FORBIDDEN_TABLES: &[&str] = &[
    "api_tokens",
    "auth_sessions",
    "auth_factors",
    "billing_events",
    "webhook_deliveries",
];

/// Whether a table is inside this avenue's remit.
#[must_use]
pub fn is_readable(table: &str) -> bool {
    READABLE_TABLES.contains(&table) && !FORBIDDEN_TABLES.contains(&table)
}

/// A trivial projection descriptor, kept for the original module contract.
#[must_use]
pub fn project() -> ReadOnlyProjection {
    ReadOnlyProjection::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_lists_never_overlap() {
        for table in READABLE_TABLES {
            assert!(!FORBIDDEN_TABLES.contains(table), "{table} is on both lists");
        }
    }

    #[test]
    fn credential_bearing_tables_are_out_of_reach() {
        assert!(!is_readable("api_tokens"));
        assert!(!is_readable("auth_sessions"));
        assert!(!is_readable("anything_else"));
        assert!(is_readable("runs"));
    }
}
