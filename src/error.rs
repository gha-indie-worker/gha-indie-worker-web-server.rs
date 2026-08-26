#![forbid(unsafe_code)]

use thiserror::Error;

#[derive(Debug, Error)]
pub enum WebError {
    #[error("unauthenticated")]
    Unauthenticated,
    #[error("unavailable")]
    Unavailable,
}

