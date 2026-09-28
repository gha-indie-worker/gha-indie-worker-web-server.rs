#![forbid(unsafe_code)]

use thiserror::Error;

#[derive(Debug, Error)]
pub enum WebError {
    #[error("unauthenticated")]
    Unauthenticated,
    #[error("unavailable")]
    Unavailable,
    #[error("invalid configuration: {0}")]
    InvalidConfiguration(&'static str),
    #[error("I/O failure: {0}")]
    Io(#[from] std::io::Error),
}
