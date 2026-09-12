#![forbid(unsafe_code)]
//! The direct read-only database avenue.
//!
//! `DATABASE_URL` names the canonical Neon project, and the role this service is given is
//! read-only. Migrations live in `gha-indie-worker-lib-core` and run as a separate one-shot job
//! with a separate credential; nothing in this binary can perform DDL, and nothing in this binary
//! should be able to.

use crate::persistence::ReadOnlyProjection;

/// A projection over many rows — the only shape of query this avenue is for.
#[must_use]
pub fn project() -> ReadOnlyProjection {
    // No pool is opened yet; the pages read fixtures. When SeaORM is wired in this returns the
    // real row count and nothing above it changes.
    ReadOnlyProjection { rows: 0 }
}

/// Whether a projection may be attempted at all.
#[must_use]
pub fn available(database_url_is_present: bool) -> bool {
    database_url_is_present
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_a_database_url_the_avenue_is_closed_rather_than_attempted() {
        assert!(!available(false));
        assert!(available(true));
        assert_eq!(project().rows, 0);
    }
}
