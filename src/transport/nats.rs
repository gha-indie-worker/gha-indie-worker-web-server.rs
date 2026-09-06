#![forbid(unsafe_code)]
//! The durable NATS avenue.
//!
//! The web tier subscribes to nothing and publishes nothing: a renderer that enqueues work is a
//! renderer that can lose it on a redeploy. Log events reach the browser through the API server,
//! which owns the subscription.

/// A subject this service would publish on, if it published.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NatsTransport {
    pub subject: String,
}

impl NatsTransport {
    #[must_use]
    pub fn new(subject: impl Into<String>) -> Self {
        Self {
            subject: subject.into(),
        }
    }

    /// The subject a run's log events are published on. Kept here so the name is written once and
    /// matches `crate::persistence::Run::log_stream`.
    #[must_use]
    pub fn run_logs(run_id: &str) -> Option<Self> {
        crate::present::is_run_id(run_id).then(|| Self::new(format!("run.{run_id}.logs")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_subject_is_only_built_from_an_identifier_that_is_one() {
        assert_eq!(
            NatsTransport::run_logs("01HZY7Q0J8").map(|t| t.subject),
            Some("run.01HZY7Q0J8.logs".to_owned())
        );
        for bad in ["", "a.b", "a b", "*", ">", "../x"] {
            assert_eq!(NatsTransport::run_logs(bad), None, "identifier {bad}");
        }
    }
}
