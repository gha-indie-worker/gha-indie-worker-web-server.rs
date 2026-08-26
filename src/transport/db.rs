#![forbid(unsafe_code)]

use crate::persistence::ReadOnlyProjection;

pub fn project() -> ReadOnlyProjection {
    ReadOnlyProjection { rows: 0 }
}

