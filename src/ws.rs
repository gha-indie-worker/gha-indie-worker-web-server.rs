#![forbid(unsafe_code)]

//! The WebSocket avenue: `/ws` on the HTTP port.
//!
//! This server does not own run state, so `/ws` is a **relay**. It accepts the
//! browser's socket, opens a second socket to the api-server's `/v1/ws` with the
//! actor's bearer attached server-side, and pumps frames between them. The
//! browser therefore never holds a credential for the api-server, and the api-
//! server keeps one authorization decision point.
//!
//! The markup side is htmx's `ws` extension: [`crate::ui::components::log_stream`]
//! renders `hx-ext="ws" ws-connect="/ws?run_id=…"`, and the relay writes HTML
//! fragments (not JSON) into the page, which is what that extension swaps.
//!
//! When the `ws-relay` feature is compiled out — or the upstream refuses — the
//! socket is closed politely and the same container falls back to htmx polling
//! against `/runs/{id}/log`. A page never loses its log view because a socket
//! could not be opened.

pub mod relay;

use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{Query, State};
use axum::response::{IntoResponse, Response};
use axum::Extension;
use serde::Deserialize;

use crate::middleware::RequestCtx;
use crate::state::AppState;

/// What the browser asked to subscribe to. Exactly one is honoured per socket.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct WsQuery {
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub chat_session: Option<String>,
}

/// A validated subscription.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Subscription {
    Run(String),
    Chat(String),
}

impl Subscription {
    /// Upstream path and query on the api-server.
    #[must_use]
    pub fn upstream_path(&self) -> String {
        match self {
            Self::Run(id) => format!("/v1/ws?run_id={}", crate::data::api_client::encode_segment(id)),
            Self::Chat(id) => format!("/v1/ws?chat_session={}", crate::data::api_client::encode_segment(id)),
        }
    }

    /// Whether a signed-in actor is required. Visitor chat is anonymous by
    /// design; run logs never are.
    #[must_use]
    pub const fn requires_actor(&self) -> bool {
        matches!(self, Self::Run(_))
    }
}

/// Identifiers come straight off a query string, so they are validated before
/// they are put anywhere near an upstream URL.
#[must_use]
pub fn parse_subscription(query: &WsQuery) -> Option<Subscription> {
    if let Some(run) = query.run_id.as_deref().filter(|id| is_safe_id(id)) {
        return Some(Subscription::Run(run.to_owned()));
    }
    query
        .chat_session
        .as_deref()
        .filter(|id| is_safe_id(id))
        .map(|id| Subscription::Chat(id.to_owned()))
}

/// A conservative identifier: 1–64 of `[A-Za-z0-9_-]`.
#[must_use]
pub fn is_safe_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 64 && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// `GET /ws` — upgrade and relay.
pub async fn handler(
    upgrade: WebSocketUpgrade,
    Query(query): Query<WsQuery>,
    Extension(context): Extension<RequestCtx>,
    State(state): State<AppState>,
) -> Response {
    let Some(subscription) = parse_subscription(&query) else {
        return crate::WebError::BadRequest("That subscription is not valid.").into_response();
    };
    if subscription.requires_actor() && context.actor.is_none() {
        return crate::WebError::Unauthenticated.into_response();
    }
    let target = format!("{}{}", state.config.api_ws_base(), subscription.upstream_path());
    let bearer = context.bearer().map(str::to_owned);
    upgrade.on_upgrade(move |socket| relay::pump(socket, target, bearer))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_are_conservative() {
        assert!(is_safe_id("run_01JABC"));
        assert!(is_safe_id("a"));
        assert!(!is_safe_id(""));
        assert!(!is_safe_id("run 1"));
        assert!(!is_safe_id("../etc"));
        assert!(!is_safe_id("run/1"));
        assert!(!is_safe_id(&"a".repeat(65)));
    }

    #[test]
    fn a_run_subscription_wins_over_a_chat_one() {
        let query = WsQuery {
            run_id: Some("run_1".into()),
            chat_session: Some("cs_1".into()),
        };
        assert_eq!(parse_subscription(&query), Some(Subscription::Run("run_1".into())));
    }

    #[test]
    fn an_invalid_identifier_yields_no_subscription() {
        let query = WsQuery {
            run_id: Some("../admin".into()),
            chat_session: None,
        };
        assert_eq!(parse_subscription(&query), None);
        assert_eq!(parse_subscription(&WsQuery::default()), None);
    }

    #[test]
    fn upstream_paths_encode_their_identifier() {
        assert_eq!(Subscription::Run("run_1".into()).upstream_path(), "/v1/ws?run_id=run_1");
        assert_eq!(
            Subscription::Chat("cs_1".into()).upstream_path(),
            "/v1/ws?chat_session=cs_1"
        );
    }

    #[test]
    fn only_run_logs_require_a_signed_in_actor() {
        assert!(Subscription::Run("r".into()).requires_actor());
        assert!(!Subscription::Chat("c".into()).requires_actor());
    }
}
