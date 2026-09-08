#![forbid(unsafe_code)]

//! The stateful TCP avenue (`GHA_INDIE_WORKER_WEB_TCP_BIND`, feature
//! `tcp-transport`).
//!
//! Length-prefixed JSON frames: a 4-byte big-endian length followed by exactly
//! that many bytes of UTF-8 JSON. The length prefix is what makes the stream
//! self-delimiting — no scanning for newlines, no ambiguity when a payload
//! contains one — and [`MAX_FRAME_BYTES`] is what stops a single frame header
//! from asking this process to allocate a gigabyte.
//!
//! This avenue is for operator tooling (`gha-indie-worker-cli`), not for
//! browsers: it is bound to a private address and is never exposed through
//! Cloudflare.

use serde::{Deserialize, Serialize};

/// Frames larger than this are refused before a single byte of body is read.
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;
/// The prefix width, in bytes.
pub const LENGTH_PREFIX_BYTES: usize = 4;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TcpTransport {
    pub bind: String,
}

impl TcpTransport {
    #[must_use]
    pub fn new(bind: impl Into<String>) -> Self {
        Self { bind: bind.into() }
    }
}

/// What an operator can ask this server over TCP. Reads only: the TCP avenue
/// deliberately cannot change anything, so exposing it to a jump host does not
/// widen the blast radius.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum TcpRequest {
    /// Liveness.
    Ping,
    /// Which surfaces this process serves, and for which base domain.
    Surfaces,
    /// Which read avenue is currently live.
    ReadSource,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(tag = "result", rename_all = "kebab-case")]
pub enum TcpResponse {
    Pong,
    Surfaces { base_domain: String, hosts: Vec<String> },
    ReadSource { avenue: String },
    Error { code: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum FrameError {
    #[error("the frame header is incomplete")]
    ShortHeader,
    #[error("the frame body is incomplete")]
    ShortBody,
    #[error("the frame exceeds the maximum size")]
    TooLarge,
    #[error("the frame is not valid JSON")]
    Malformed,
}

/// Encodes one frame: a 4-byte big-endian length, then the JSON body.
pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, FrameError> {
    let body = serde_json::to_vec(value).map_err(|_| FrameError::Malformed)?;
    if body.len() > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge);
    }
    let length = u32::try_from(body.len()).map_err(|_| FrameError::TooLarge)?;
    let mut frame = Vec::with_capacity(LENGTH_PREFIX_BYTES + body.len());
    frame.extend_from_slice(&length.to_be_bytes());
    frame.extend_from_slice(&body);
    Ok(frame)
}

/// Reads the declared body length from a header, refusing anything oversized
/// **before** the body is allocated.
pub fn frame_length(header: &[u8]) -> Result<usize, FrameError> {
    let bytes: [u8; LENGTH_PREFIX_BYTES] = header
        .get(..LENGTH_PREFIX_BYTES)
        .ok_or(FrameError::ShortHeader)?
        .try_into()
        .map_err(|_| FrameError::ShortHeader)?;
    let length = usize::try_from(u32::from_be_bytes(bytes)).map_err(|_| FrameError::TooLarge)?;
    if length > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge);
    }
    Ok(length)
}

/// Decodes one complete frame (header + body).
pub fn decode<T: for<'de> Deserialize<'de>>(frame: &[u8]) -> Result<T, FrameError> {
    let length = frame_length(frame)?;
    let body = frame
        .get(LENGTH_PREFIX_BYTES..LENGTH_PREFIX_BYTES + length)
        .ok_or(FrameError::ShortBody)?;
    serde_json::from_slice(body).map_err(|_| FrameError::Malformed)
}

/// Answers one request. Pure, so the protocol is testable without a socket.
#[must_use]
pub fn handle(request: &TcpRequest, base_domain: &str, read_source: &str) -> TcpResponse {
    match request {
        TcpRequest::Ping => TcpResponse::Pong,
        TcpRequest::Surfaces => TcpResponse::Surfaces {
            base_domain: base_domain.to_owned(),
            hosts: [
                crate::hosts::Surface::App,
                crate::hosts::Surface::User,
                crate::hosts::Surface::Org,
                crate::hosts::Surface::Mobile,
            ]
            .iter()
            .map(|surface| surface.host(base_domain))
            .collect(),
        },
        TcpRequest::ReadSource => TcpResponse::ReadSource {
            avenue: read_source.to_owned(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip() {
        let frame = encode(&TcpRequest::Ping).expect("encodes");
        assert_eq!(frame_length(&frame).expect("length"), frame.len() - LENGTH_PREFIX_BYTES);
        assert_eq!(decode::<TcpRequest>(&frame).expect("decodes"), TcpRequest::Ping);
    }

    #[test]
    fn an_oversized_header_is_refused_before_any_body_is_read() {
        let header = u32::MAX.to_be_bytes();
        assert_eq!(frame_length(&header), Err(FrameError::TooLarge));
    }

    #[test]
    fn a_truncated_frame_is_reported_rather_than_panicking() {
        assert_eq!(frame_length(&[0, 0]), Err(FrameError::ShortHeader));
        let mut frame = encode(&TcpRequest::Ping).expect("encodes");
        frame.pop();
        assert_eq!(decode::<TcpRequest>(&frame), Err(FrameError::ShortBody));
    }

    #[test]
    fn a_malformed_body_is_reported() {
        let mut frame = vec![0, 0, 0, 3];
        frame.extend_from_slice(b"not");
        assert_eq!(decode::<TcpRequest>(&frame), Err(FrameError::Malformed));
    }

    #[test]
    fn surfaces_are_reported_for_the_configured_base_domain() {
        let response = handle(&TcpRequest::Surfaces, "indiebuild.dev", "database");
        match response {
            TcpResponse::Surfaces { base_domain, hosts } => {
                assert_eq!(base_domain, "indiebuild.dev");
                assert_eq!(
                    hosts,
                    vec![
                        "app.indiebuild.dev".to_owned(),
                        "user.indiebuild.dev".to_owned(),
                        "org.indiebuild.dev".to_owned(),
                        "m.indiebuild.dev".to_owned(),
                    ]
                );
            }
            other => panic!("unexpected response: {other:?}"),
        }
    }

    #[test]
    fn ping_and_read_source_answer_without_touching_state() {
        assert_eq!(handle(&TcpRequest::Ping, "d", "http"), TcpResponse::Pong);
        assert_eq!(
            handle(&TcpRequest::ReadSource, "d", "http"),
            TcpResponse::ReadSource {
                avenue: "http".to_owned()
            }
        );
    }
}
