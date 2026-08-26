#![forbid(unsafe_code)]

/// Direct read-only ORM access. Migrations stay in lib-core.

#[derive(Clone, Debug, Default)]
pub struct ReadOnlyProjection {
    pub rows: usize,
}

