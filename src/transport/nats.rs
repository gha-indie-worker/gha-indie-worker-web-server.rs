#![forbid(unsafe_code)]

//! The async NATS/JetStream avenue (`GHA_INDIE_WORKER_NATS_URL`, feature
//! `nats-transport`). Two things share the broker and are kept apart here:
//!
//! 1. **The web → API request plane.** A pure envelope and subject builder for
//!    `dd.remote.web_api.gha-indie-worker.request`: [`RequestEnvelope`] is
//!    validated and serialized by [`publish_request`] without touching a broker.
//!    Connecting and publishing over the wire is an effect at the process
//!    boundary. Credentials are never CLI flags; they stay in the environment or
//!    secret store that supplies `GHA_INDIE_WORKER_NATS_URL`.
//!
//! 2. **Intent events.** Subjects follow the fleet algebra
//!    `giw.<env>.<domain>.<event>`, which this module owns and validates. The web
//!    server publishes what a person did on a page (a run was re-run, an
//!    invitation was sent) for anything downstream that cares. It never
//!    consumes, because a page render must not depend on a queue.
//!
//! The `async-nats` client itself is wired in the api-server, which owns the
//! connection and the JetStream contexts; this module is the subject algebra and
//! the envelope shapes, so both servers agree on the wire without this crate
//! taking on a heavyweight dependency it would use for a handful of publishes.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

pub const REQUEST_SUBJECT: &str = "dd.remote.web_api.gha-indie-worker.request";
pub const STATUS_SUBJECT: &str = "dd.remote.web_api.gha-indie-worker.status";
pub const CONTRACT: &str = "gha-indie-worker/web-api/v1";
pub const AUDIENCE: &str = "gha-indie-worker-api";
pub const MAX_REQUEST_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractionMode {
    DirectReadOnlyDatabase,
    StatelessHttp,
    StatefulMtlsTcp,
    Nats,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Read,
    Write,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum NatsError {
    #[error("NATS URL is required when mode is nats")]
    MissingUrl,
    #[error("invalid NATS URL")]
    InvalidUrl,
    #[error("unsupported web/API contract")]
    Contract,
    #[error("wrong API audience")]
    Audience,
    #[error("invalid {0}")]
    InvalidIdentifier(&'static str),
    #[error("invalid resource")]
    InvalidResource,
    #[error("NATS mode requires a dedupe key")]
    MissingDedupeKey,
    #[error("request exceeds the byte limit")]
    RequestTooLarge,
    #[error("serialization failed")]
    Serialization,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestEnvelope {
    pub contract: String,
    pub request_id: String,
    pub tenant_id: String,
    pub subject: String,
    pub audience: String,
    pub operation: Operation,
    pub resource: String,
    pub payload: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dedupe_key: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublishRequest {
    pub subject: &'static str,
    pub payload: Vec<u8>,
}

pub fn require_nats_url(
    mode: InteractionMode,
    url: Option<&str>,
) -> Result<Option<&str>, NatsError> {
    let trimmed = url.map(str::trim).filter(|value| !value.is_empty());
    match (mode, trimmed) {
        (InteractionMode::Nats, None) => Err(NatsError::MissingUrl),
        (_, Some(url)) if valid_nats_url(url) => Ok(Some(url)),
        (_, Some(_)) => Err(NatsError::InvalidUrl),
        (_, None) => Ok(None),
    }
}

/// Build the request subject and serialized envelope without touching a broker.
pub fn publish_request(
    url: Option<&str>,
    mode: InteractionMode,
    envelope: &RequestEnvelope,
) -> Result<PublishRequest, NatsError> {
    if mode == InteractionMode::Nats {
        require_nats_url(mode, url)?;
    }
    envelope.validate_for(mode)?;
    let payload = serde_json::to_vec(envelope).map_err(|_| NatsError::Serialization)?;
    if payload.len() > MAX_REQUEST_BYTES {
        return Err(NatsError::RequestTooLarge);
    }
    Ok(PublishRequest {
        subject: REQUEST_SUBJECT,
        payload,
    })
}

impl RequestEnvelope {
    pub fn validate_for(&self, mode: InteractionMode) -> Result<(), NatsError> {
        if self.contract != CONTRACT {
            return Err(NatsError::Contract);
        }
        validate_identifier("request_id", &self.request_id, 128)?;
        validate_identifier("tenant_id", &self.tenant_id, 128)?;
        validate_identifier("subject", &self.subject, 255)?;
        if self.audience != AUDIENCE {
            return Err(NatsError::Audience);
        }
        validate_resource(&self.resource)?;
        if mode == InteractionMode::Nats {
            validate_dedupe_key(self.dedupe_key.as_deref())?;
        }
        Ok(())
    }
}

fn valid_nats_url(value: &str) -> bool {
    let Some(rest) = value
        .strip_prefix("nats://")
        .or_else(|| value.strip_prefix("tls://"))
    else {
        return false;
    };
    if rest.is_empty() || rest.contains(char::is_whitespace) {
        return false;
    }
    let host = rest.rsplit_once('@').map(|(_, host)| host).unwrap_or(rest);
    !host.is_empty()
        && (host.contains('.')
            || host.starts_with("127.0.0.1")
            || host.starts_with("localhost")
            || host.starts_with('['))
}

fn validate_identifier(field: &'static str, value: &str, maximum: usize) -> Result<(), NatsError> {
    if value.is_empty()
        || value.len() > maximum
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err(NatsError::InvalidIdentifier(field));
    }
    Ok(())
}

fn validate_resource(value: &str) -> Result<(), NatsError> {
    if value.is_empty()
        || value.len() > 256
        || !value.starts_with('/')
        || value.contains("..")
        || value.contains(['?', '#'])
        || value.chars().any(char::is_control)
    {
        return Err(NatsError::InvalidResource);
    }
    Ok(())
}

fn validate_dedupe_key(value: Option<&str>) -> Result<(), NatsError> {
    let value = value.ok_or(NatsError::MissingDedupeKey)?;
    if value.len() < 8 {
        return Err(NatsError::MissingDedupeKey);
    }
    validate_identifier("dedupe_key", value, 128)
}

// ---------------------------------------------------------------------------
// Intent events: giw.<env>.<domain>.<event>
// ---------------------------------------------------------------------------

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
    use serde_json::json;

    fn envelope(operation: Operation) -> RequestEnvelope {
        RequestEnvelope {
            contract: CONTRACT.into(),
            request_id: "request-01".into(),
            tenant_id: "tenant-01".into(),
            subject: "11111111-1111-4111-8111-111111111111".into(),
            audience: AUDIENCE.into(),
            operation,
            resource: "/builds".into(),
            payload: json!({"limit": 25}),
            dedupe_key: Some("tenant-01:request-01".into()),
        }
    }

    #[test]
    fn nats_mode_fails_closed_without_a_url() {
        assert_eq!(
            require_nats_url(InteractionMode::Nats, None),
            Err(NatsError::MissingUrl)
        );
        assert_eq!(
            require_nats_url(InteractionMode::Nats, Some("   ")),
            Err(NatsError::MissingUrl)
        );
        assert_eq!(
            require_nats_url(InteractionMode::StatelessHttp, None),
            Ok(None)
        );
        assert_eq!(
            publish_request(None, InteractionMode::Nats, &envelope(Operation::Write)),
            Err(NatsError::MissingUrl)
        );
    }

    #[test]
    fn publish_request_is_a_pure_envelope_on_the_org_subject() {
        let planned = publish_request(
            Some("nats://127.0.0.1:4222"),
            InteractionMode::Nats,
            &envelope(Operation::Write),
        )
        .expect("valid NATS envelope");

        assert_eq!(planned.subject, REQUEST_SUBJECT);
        assert_eq!(
            planned.subject,
            "dd.remote.web_api.gha-indie-worker.request"
        );

        let decoded: RequestEnvelope =
            serde_json::from_slice(&planned.payload).expect("round-trip");
        assert_eq!(decoded, envelope(Operation::Write));
        assert!(!String::from_utf8_lossy(&planned.payload).contains("nats://"));
    }

    #[test]
    fn nats_envelope_requires_a_dedupe_key() {
        let mut missing = envelope(Operation::Write);
        missing.dedupe_key = None;
        assert_eq!(
            publish_request(
                Some("nats://127.0.0.1:4222"),
                InteractionMode::Nats,
                &missing
            ),
            Err(NatsError::MissingDedupeKey)
        );
    }

    #[test]
    fn credentials_never_appear_as_typed_nats_fields() {
        let planned = publish_request(
            Some("nats://worker:credential@nats.internal:4222"),
            InteractionMode::Nats,
            &envelope(Operation::Read),
        )
        .expect("URL userinfo is transport config, not envelope data");
        let decoded: RequestEnvelope =
            serde_json::from_slice(&planned.payload).expect("round-trip");
        let encoded = serde_json::to_value(&decoded).expect("json");
        assert!(encoded.get("password").is_none());
        assert!(encoded.get("token").is_none());
        assert!(encoded.get("nats_url").is_none());
        assert!(!format!("{decoded:?}").contains("credential"));
    }

    // -- intent events --

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
