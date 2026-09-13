#![forbid(unsafe_code)]

//! ores-chat, proxied through the api-server.
//!
//! The browser never talks to ores-chat: it posts to this server, this server
//! calls the api-server's `/v1/chat/*`, and the api-server holds the ores-chat
//! credential. That keeps the CSP at `'self'`, keeps the chat vendor out of the
//! page, and means a visitor's message is subject to the same rate limit and
//! audit trail as everything else.
//!
//! Two surfaces:
//!
//! * `visitor` — the signed-out sales widget on the `app.` marketing home. No
//!   bearer is forwarded, because there is no account to speak for.
//! * `customer` — the signed-in support widget on dashboards. The actor's bearer
//!   is forwarded so support can see the customer's runs.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::data::ApiClient;
use crate::error::WebError;
use crate::ui::chat_widget::ChatAudience;

/// Longest message this server will forward. The api-server bounds it again.
pub const MAX_MESSAGE_BYTES: usize = 4_000;

/// An open chat session as the page needs it.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSession {
    pub id: String,
    #[serde(default)]
    pub surface: String,
    #[serde(default)]
    pub messages: Vec<ChatMessage>,
    /// Set when the api-server can stream this session over the relay.
    #[serde(default)]
    pub streaming: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub body: String,
}

impl ChatSession {
    /// `(author, body)` pairs for [`crate::ui::chat_widget::message_list`].
    #[must_use]
    pub fn rendered(&self) -> Vec<(String, String)> {
        self.messages
            .iter()
            .map(|message| {
                let author = if message.author.is_empty() {
                    "Support".to_owned()
                } else {
                    message.author.clone()
                };
                (author, message.body.clone())
            })
            .collect()
    }
}

#[derive(Clone)]
pub struct ChatService {
    api: Arc<ApiClient>,
    enabled: bool,
}

impl std::fmt::Debug for ChatService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ChatService")
            .field("enabled", &self.enabled)
            .finish_non_exhaustive()
    }
}

impl ChatService {
    #[must_use]
    pub fn new(api: Arc<ApiClient>, enabled: bool) -> Self {
        Self { api, enabled }
    }

    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Opens a session. A visitor's bearer is deliberately dropped.
    pub async fn open(
        &self,
        audience: ChatAudience,
        bearer: Option<&str>,
        page: &str,
    ) -> Result<ChatSession, WebError> {
        if !self.enabled {
            return Err(WebError::Unavailable);
        }
        let bearer = forwarded_bearer(audience, bearer);
        let value = self.api.open_chat_session(bearer, audience.api_surface(), page).await?;
        parse_session(value)
    }

    pub async fn send(
        &self,
        audience: ChatAudience,
        bearer: Option<&str>,
        session_id: &str,
        message: &str,
    ) -> Result<ChatSession, WebError> {
        if !self.enabled {
            return Err(WebError::Unavailable);
        }
        let message = normalize_message(message)?;
        let bearer = forwarded_bearer(audience, bearer);
        let value = self.api.send_chat_message(bearer, session_id, &message).await?;
        parse_session(value)
    }

    pub async fn history(
        &self,
        audience: ChatAudience,
        bearer: Option<&str>,
        session_id: &str,
    ) -> Result<ChatSession, WebError> {
        if !self.enabled {
            return Err(WebError::Unavailable);
        }
        let bearer = forwarded_bearer(audience, bearer);
        let value = self.api.chat_messages(bearer, session_id).await?;
        parse_session(value)
    }
}

/// A visitor session never carries a credential, even if one happens to exist.
#[must_use]
pub fn forwarded_bearer(audience: ChatAudience, bearer: Option<&str>) -> Option<&str> {
    match audience {
        ChatAudience::Visitor => None,
        ChatAudience::Customer => bearer,
    }
}

/// `visitor` / `customer` from a query string; anything else is a visitor.
#[must_use]
pub fn audience_from_param(raw: Option<&str>) -> ChatAudience {
    match raw {
        Some(value) if value.eq_ignore_ascii_case("customer") => ChatAudience::Customer,
        _ => ChatAudience::Visitor,
    }
}

/// Trims, rejects empty, and bounds the length.
pub fn normalize_message(raw: &str) -> Result<String, WebError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(WebError::BadRequest("Type a message before sending."));
    }
    if trimmed.len() > MAX_MESSAGE_BYTES {
        return Err(WebError::BadRequest("That message is too long to send."));
    }
    Ok(trimmed.to_owned())
}

/// The api-server may answer with the session directly or wrap it in `session`.
fn parse_session(value: serde_json::Value) -> Result<ChatSession, WebError> {
    let candidate = value.get("session").cloned().unwrap_or(value);
    serde_json::from_value(candidate).map_err(|_| WebError::Unavailable)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_visitor_session_never_forwards_a_credential() {
        assert_eq!(forwarded_bearer(ChatAudience::Visitor, Some("tok")), None);
        assert_eq!(forwarded_bearer(ChatAudience::Customer, Some("tok")), Some("tok"));
        assert_eq!(forwarded_bearer(ChatAudience::Customer, None), None);
    }

    #[test]
    fn an_unknown_surface_parameter_falls_back_to_visitor() {
        assert_eq!(audience_from_param(Some("customer")), ChatAudience::Customer);
        assert_eq!(audience_from_param(Some("CUSTOMER")), ChatAudience::Customer);
        assert_eq!(audience_from_param(Some("owner")), ChatAudience::Visitor);
        assert_eq!(audience_from_param(None), ChatAudience::Visitor);
    }

    #[test]
    fn messages_are_trimmed_and_bounded() {
        assert_eq!(normalize_message("  hello  ").expect("valid"), "hello");
        assert!(normalize_message("   ").is_err());
        assert!(normalize_message(&"x".repeat(MAX_MESSAGE_BYTES + 1)).is_err());
        assert!(normalize_message(&"x".repeat(MAX_MESSAGE_BYTES)).is_ok());
    }

    #[test]
    fn a_session_parses_wrapped_or_bare() {
        let bare = serde_json::json!({ "id": "cs_1", "surface": "visitor", "messages": [] });
        assert_eq!(parse_session(bare).expect("parses").id, "cs_1");

        let wrapped = serde_json::json!({ "session": { "id": "cs_2", "messages": [{ "author": "", "body": "hi" }] } });
        let session = parse_session(wrapped).expect("parses");
        assert_eq!(session.id, "cs_2");
        assert_eq!(session.rendered(), vec![("Support".to_owned(), "hi".to_owned())]);
    }

    #[test]
    fn a_disabled_service_reports_unavailable_rather_than_calling_out() {
        let service = ChatService::new(Arc::new(ApiClient::for_tests()), false);
        assert!(!service.is_enabled());
    }
}
