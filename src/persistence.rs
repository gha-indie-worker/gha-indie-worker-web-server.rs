#![forbid(unsafe_code)]

//! Persistence policy for the web surfaces.
//!
//! This server **reads**. It never writes, never runs DDL, and never holds a
//! migration: migrations belong to `gha-indie-worker-lib-core`, and writes
//! belong to the api-server. The credential in `DATABASE_URL_CANONICAL` is a
//! read-only role, and [`crate::data::db::ReadPool`] refuses any statement that
//! is not a `SELECT` — belt and braces, because a read-only role can be widened
//! by a well-meaning migration and a compiled-in check cannot.

/// The shape a projection query reports back.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ReadOnlyProjection {
    pub rows: usize,
}

impl ReadOnlyProjection {
    #[must_use]
    pub const fn of(rows: usize) -> Self {
        Self { rows }
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.rows == 0
    }
}

/// What this process is permitted to do to the canonical database.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Capability {
    Read,
    Write,
    Migrate,
}

/// The answer is `Read`, and only `Read`.
#[must_use]
pub const fn is_permitted(capability: Capability) -> bool {
    matches!(capability, Capability::Read)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_web_server_may_only_read() {
        assert!(is_permitted(Capability::Read));
        assert!(!is_permitted(Capability::Write));
        assert!(!is_permitted(Capability::Migrate));
    }

    #[test]
    fn an_empty_projection_says_so() {
        assert!(ReadOnlyProjection::default().is_empty());
        assert!(!ReadOnlyProjection::of(1).is_empty());
    }
}
