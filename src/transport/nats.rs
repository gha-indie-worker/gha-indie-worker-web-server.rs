#![forbid(unsafe_code)]

//! The async NATS/JetStream avenue (`GHA_INDIE_WORKER_NATS_URL`, feature
//! `nats-transport`).
//!
//! Subjects follow the fleet algebra `giw.<env>.<domain>.<event>`, which this
//! module owns and validates. The web server is a **publisher of intent only**:
//! it emits what a person did on a page (a run was re-run, an invitation was
//! sent) for anything downstream that cares. It never consumes, because a page
//! render must not depend on a queue.
//!
//! The `async-nats` client itself is wired in the api-server, which owns the
//! connection and the JetStream contexts; this module is the subject algebra and
//! the envelope shape, so both servers agree on the wire without this crate
//! taking on a heavyweight dependency it would use for a handful of publishes.

use serde::{Deserialize, Serialize};

/// Root of every subject this org publishes.
pub const SUBJECT_ROOT: &str = "giw";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NatsTransport {
    pub url: String,
    pub environment: String,
}

impl NatsTransport {
    #[must_use]
    pub fn new(url: impl Into<String>, environment: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            environment: environment.into(),
        }
    }

    /// `giw.production.runs.rerun-requested`
    #[must_use]
    pub fn subject(&self, domain: &str, event: &str) -> String {
        format!("{SUBJECT_ROOT}.{}.{}.{}", self.environment, domain, event)
    }
}

/// The envelope every publish carries. `causation_id` is the request id from
/// ores-middleware, so a page action can be traced end to end.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Envelope<T> {
    pub subject: String,
    pub causation_id: String,
    pub occurred_at: String,
    pub payload: T,
}

impl<T> Envelope<T> {
    #[must_use]
    pub fn new(
        subject: impl Into<String>,
        causation_id: impl Into<String>,
        occurred_at: impl Into<String>,
        payload: T,
    ) -> Self {
        Self {
            subject: subject.into(),
            causation_id: causation_id.into(),
            occurred_at: occurred_at.into(),
            payload,
        }
    }
}

/// A subject segment must be a lowercase token: no wildcards, no separators, so
/// a caller-supplied value can never widen a subscription.
#[must_use]
pub fn is_valid_segment(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Builds a subject, refusing anything that is not a plain segment.
#[must_use]
pub fn subject(environment: &str, domain: &str, event: &str) -> Option<String> {
    (is_valid_segment(environment) && is_valid_segment(domain) && is_valid_segment(event))
        .then(|| format!("{SUBJECT_ROOT}.{environment}.{domain}.{event}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subjects_follow_the_fleet_algebra() {
        assert_eq!(
            subject("production", "runs", "rerun-requested").as_deref(),
            Some("giw.production.runs.rerun-requested")
        );
        assert_eq!(
            NatsTransport::new("nats://localhost:4222", "staging").subject("orgs", "invite-sent"),
            "giw.staging.orgs.invite-sent"
        );
    }

    #[test]
    fn a_wildcard_can_never_reach_a_subject() {
        assert!(!is_valid_segment("*"));
        assert!(!is_valid_segment(">"));
        assert!(!is_valid_segment("runs.secret"));
        assert!(!is_valid_segment("Runs"));
        assert!(!is_valid_segment(""));
        assert_eq!(subject("production", "runs", ">"), None);
    }

    #[test]
    fn envelopes_serialize_with_the_fleet_field_names() {
        let envelope = Envelope::new("giw.test.runs.x", "req_1", "2026-01-01T00:00:00Z", 7u32);
        let json = serde_json::to_value(&envelope).expect("serializes");
        assert_eq!(json["causationId"], "req_1");
        assert_eq!(json["occurredAt"], "2026-01-01T00:00:00Z");
        assert_eq!(json["payload"], 7);
    }
}
