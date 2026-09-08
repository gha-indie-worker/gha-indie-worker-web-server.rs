#![forbid(unsafe_code)]

//! The health payload.
//!
//! `/healthz` is served by [`crate::hosts::common`] on every product surface and
//! is deliberately **not** mounted on the unknown-host surface. The body is
//! generated here so the shape has one definition and one test.

use serde::Serialize;

/// What `/healthz` answers. Nothing here depends on a database, an upstream, or
/// a credential: liveness must not fail because a dependency is having a day.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Health {
    pub status: &'static str,
    pub service: &'static str,
    pub namespace: &'static str,
}

impl Default for Health {
    fn default() -> Self {
        Self {
            status: "ok",
            service: crate::SERVICE_NAME,
            namespace: crate::SERVICE_NAMESPACE,
        }
    }
}

/// The health payload as JSON.
#[must_use]
pub fn json() -> String {
    serde_json::to_string(&Health::default()).unwrap_or_else(|_| r#"{"status":"ok"}"#.to_owned())
}

/// A human-readable one-liner, kept for the original module contract.
#[must_use]
pub fn markup() -> String {
    format!("<p>{} health ok</p>", crate::SERVICE_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_payload_names_the_service_and_namespace() {
        let value: serde_json::Value = serde_json::from_str(&json()).expect("valid json");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["service"], crate::SERVICE_NAME);
        assert_eq!(value["namespace"], "gha-indie-worker");
    }

    #[test]
    fn the_payload_carries_nothing_sensitive() {
        let text = json();
        assert!(!text.contains("postgres"));
        assert!(!text.contains("secret"));
        assert!(!text.contains("token"));
    }

    #[test]
    fn the_markup_form_is_still_available() {
        assert!(markup().contains("health ok"));
    }
}
