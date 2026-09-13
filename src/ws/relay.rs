#![forbid(unsafe_code)]

//! The relay itself, isolated so the tokio-tungstenite dependency has exactly
//! one point of contact with this crate.
//!
//! Frames are copied verbatim in both directions. Text is the only payload the
//! product uses (htmx's `ws` extension swaps HTML fragments), so binary frames
//! are dropped rather than forwarded — a browser has no use for them here, and
//! not forwarding them keeps the memory ceiling obvious.
//!
//! With the `ws-relay` feature off, [`pump`] closes the socket with a short
//! message and the page falls back to htmx polling.

use axum::extract::ws::{Message, WebSocket};
use futures_util::SinkExt;

/// Sent to the browser when the relay cannot be established, so the page can
/// say something true instead of showing an empty log.
pub const UNAVAILABLE_FRAGMENT: &str =
    r#"<span class="line" id="log-stream-notice">Live streaming is unavailable; falling back to polling.</span>"#;

/// Relays one browser socket to `target` on the api-server.
pub async fn pump(socket: WebSocket, target: String, bearer: Option<String>) {
    #[cfg(feature = "ws-relay")]
    {
        if let Err(error) = relay(socket, &target, bearer.as_deref()).await {
            tracing::debug!(relay.outcome = %error, "websocket relay ended");
        }
    }
    #[cfg(not(feature = "ws-relay"))]
    {
        let _ = (target, bearer);
        close_with_notice(socket).await;
    }
}

/// Tells the browser to fall back, then closes cleanly.
pub async fn close_with_notice(mut socket: WebSocket) {
    let _ = socket.send(Message::Text(UNAVAILABLE_FRAGMENT.to_owned().into())).await;
    let _ = socket.close().await;
}

#[cfg(feature = "ws-relay")]
mod upstream {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::http::{HeaderName, HeaderValue};

    use super::RelayError;

    /// Opens the upstream socket with the bearer attached **server-side**. The
    /// browser never sees or holds this credential.
    pub async fn connect(
        target: &str,
        bearer: Option<&str>,
    ) -> Result<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, RelayError>
    {
        let mut request = target.into_client_request().map_err(|_| RelayError::Target)?;
        if let Some(token) = bearer {
            let value = HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| RelayError::Target)?;
            let name = HeaderName::from_static("authorization");
            request.headers_mut().insert(name, value);
        }
        let (stream, _response) = tokio_tungstenite::connect_async(request)
            .await
            .map_err(|_| RelayError::Upstream)?;
        Ok(stream)
    }
}

/// Why a relay stopped. Never carries an upstream message.
#[derive(Clone, Copy, Debug, thiserror::Error)]
pub enum RelayError {
    #[error("the upstream target is not a valid websocket URL")]
    Target,
    #[error("the api-server websocket could not be opened")]
    Upstream,
}

#[cfg(feature = "ws-relay")]
async fn relay(socket: WebSocket, target: &str, bearer: Option<&str>) -> Result<(), RelayError> {
    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite::Message as UpstreamMessage;

    let upstream = match upstream::connect(target, bearer).await {
        Ok(stream) => stream,
        Err(error) => {
            close_with_notice(socket).await;
            return Err(error);
        }
    };

    let (mut browser_tx, mut browser_rx) = socket.split();
    let (mut upstream_tx, mut upstream_rx) = upstream.split();

    // api-server → browser
    let downstream = async move {
        while let Some(Ok(frame)) = upstream_rx.next().await {
            match frame {
                UpstreamMessage::Text(text) => {
                    if browser_tx.send(Message::Text(text.to_string().into())).await.is_err() {
                        break;
                    }
                }
                UpstreamMessage::Close(_) => break,
                // Binary, ping and pong are not part of this product's protocol.
                _ => {}
            }
        }
        let _ = browser_tx.close().await;
    };

    // browser → api-server
    let upstream_pump = async move {
        while let Some(Ok(frame)) = browser_rx.next().await {
            match frame {
                Message::Text(text) => {
                    if upstream_tx
                        .send(UpstreamMessage::Text(text.to_string().into()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
        let _ = upstream_tx.close().await;
    };

    // Either direction ending ends the relay: a half-open socket is a leak.
    tokio::select! {
        () = downstream => {}
        () = upstream_pump => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fallback_notice_is_an_htmx_swappable_fragment() {
        assert!(UNAVAILABLE_FRAGMENT.starts_with("<span"));
        assert!(UNAVAILABLE_FRAGMENT.contains("log-stream-notice"));
        assert!(!UNAVAILABLE_FRAGMENT.contains("<script"));
    }

    #[test]
    fn relay_errors_never_carry_an_upstream_message() {
        for error in [RelayError::Target, RelayError::Upstream] {
            let text = error.to_string();
            assert!(!text.contains("http"));
            assert!(!text.contains("Bearer"));
        }
    }
}
