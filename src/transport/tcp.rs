#![forbid(unsafe_code)]
//! The stateful mTLS TCP avenue.
//!
//! The web tier does not open one. It is declared here because
//! [`crate::transport::choose`] names it, and because a module that exists and says "not from
//! here" is clearer than a module that is missing.

/// Where a stateful session would be opened.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TcpTransport {
    pub bind: String,
}

impl TcpTransport {
    #[must_use]
    pub fn new(bind: impl Into<String>) -> Self {
        Self { bind: bind.into() }
    }

    /// Always false in this binary: runners talk to the API server, never to the renderer.
    #[must_use]
    pub const fn available_from_the_web_tier() -> bool {
        false
    }
}
