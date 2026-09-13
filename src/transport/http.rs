#![forbid(unsafe_code)]

//! The stateless HTTP avenue.
//!
//! The client itself is [`crate::data::api_client::ApiClient`]; this module is
//! the avenue's descriptor — the base URL and the derived WebSocket origin, in
//! one place so the relay and the JSON calls can never disagree about which
//! api-server they are talking to.

/// Where this server's HTTP avenue points.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HttpTransport {
    pub base: String,
}

impl HttpTransport {
    #[must_use]
    pub fn new(base: impl Into<String>) -> Self {
        Self {
            base: base.into().trim_end_matches('/').to_owned(),
        }
    }

    /// The matching WebSocket origin: `https` → `wss`, `http` → `ws`.
    #[must_use]
    pub fn websocket_base(&self) -> String {
        if let Some(rest) = self.base.strip_prefix("https://") {
            format!("wss://{rest}")
        } else if let Some(rest) = self.base.strip_prefix("http://") {
            format!("ws://{rest}")
        } else {
            format!("wss://{}", self.base)
        }
    }

    #[must_use]
    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    /// Cleartext to anything but loopback is a configuration error.
    #[must_use]
    pub fn is_safe_for_production(&self) -> bool {
        self.base.starts_with("https://")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trailing_slashes_never_double_up() {
        let transport = HttpTransport::new("https://api.indiebuild.dev/");
        assert_eq!(transport.url("/v1/runs"), "https://api.indiebuild.dev/v1/runs");
    }

    #[test]
    fn the_websocket_origin_follows_the_http_scheme() {
        assert_eq!(
            HttpTransport::new("https://api.indiebuild.dev").websocket_base(),
            "wss://api.indiebuild.dev"
        );
        assert_eq!(
            HttpTransport::new("http://127.0.0.1:8080").websocket_base(),
            "ws://127.0.0.1:8080"
        );
    }

    #[test]
    fn cleartext_is_not_production_safe() {
        assert!(HttpTransport::new("https://api.indiebuild.dev").is_safe_for_production());
        assert!(!HttpTransport::new("http://api.indiebuild.dev").is_safe_for_production());
    }
}
