//! The WebSocket wire format — the pure half.
//!
//! `lib_core::runtime::session` defines one [frame vocabulary][v] with two encodings: a binary
//! one for the raw-TCP transport, and JSON text for browsers. lib-core ships the binary codec;
//! this file is the JSON one, written here because only the browser transport needs it.
//!
//! [v]: https://github.com/gha-indie-worker/gha-indie-worker-lib-core
//!
//! It imports nothing, and it speaks a *mirror* of the frame enum rather than the real one. The
//! conversion between the two lives in [`crate::bridge`], where the compiler checks the match is
//! exhaustive — so a frame added to lib-core cannot quietly become unencodable, and this file
//! still compiles and tests on its own:
//!
//! ```text
//! rustc --edition 2021 --test src/wire.rs -o /tmp/wire && /tmp/wire
//! ```
//!
//! Three properties the parser is built around, because this is the one place a stranger's bytes
//! reach us before anything has authenticated them:
//!
//! 1. **Frames are flat.** A frame is a JSON object of scalars — no nested objects, no arrays.
//!    The parser refuses anything else outright, which removes recursion, and with it the whole
//!    category of depth-based denial of service.
//! 2. **Everything is bounded.** Input length, key count, string length, and number length all
//!    have limits, and exceeding one is a refusal rather than an allocation.
//! 3. **Payloads stay readable.** A log line is UTF-8, so it is written as a JSON string that a
//!    person can read in devtools; bytes that are not UTF-8 are written as base64url under a
//!    different key. The decoder accepts either, and never both.

/// The largest JSON frame accepted, matching `session::MAX_FRAME_BYTES`.
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;
/// The largest single string inside a frame.
pub const MAX_STRING_BYTES: usize = 256 * 1024;
/// A frame has ten fields at the very most; anything longer is not one of ours.
pub const MAX_KEYS: usize = 16;

/// Mirror of `session::Frame`. One variant per real variant; see [`crate::bridge`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WireFrame {
    Hello {
        version: u16,
        resume: Option<String>,
    },
    Welcome {
        session: String,
        resumed: bool,
        heartbeat_seconds: u32,
    },
    Ping {
        nonce: u64,
    },
    Pong {
        nonce: u64,
    },
    Subscribe {
        stream: String,
        from: u64,
        credit: u32,
    },
    Unsubscribe {
        stream: String,
    },
    Credit {
        stream: String,
        additional: u32,
    },
    Event {
        stream: String,
        sequence: u64,
        payload: Vec<u8>,
    },
    Lagged {
        stream: String,
        skipped_to: u64,
    },
    Command {
        id: u64,
        verb: String,
        payload: Vec<u8>,
    },
    Ack {
        id: u64,
        payload: Vec<u8>,
    },
    Error {
        id: Option<u64>,
        code: String,
        message: String,
    },
    Close {
        code: String,
        message: String,
    },
}

/// Why a text frame was not a frame. Every variant closes the connection; none is described to
/// the peer in more detail than `session::ErrorCode::Protocol`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WireError {
    /// Longer than [`MAX_FRAME_BYTES`], or a string or key count over its own limit.
    TooLarge,
    /// Not a flat JSON object, or a value of a kind frames do not contain.
    NotAFrame,
    /// A `type` we do not speak.
    UnknownType,
    /// A field is absent, or present with the wrong kind of value.
    BadField,
}

// ---------------------------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------------------------

/// Render a frame as one line of JSON.
#[must_use]
pub fn encode(frame: &WireFrame) -> String {
    let mut out = String::with_capacity(128);
    out.push('{');
    match frame {
        WireFrame::Hello { version, resume } => {
            tag(&mut out, "hello");
            number(&mut out, "version", u64::from(*version));
            match resume {
                Some(session) => string(&mut out, "resume", session),
                None => out.push_str(",\"resume\":null"),
            }
        }
        WireFrame::Welcome {
            session,
            resumed,
            heartbeat_seconds,
        } => {
            tag(&mut out, "welcome");
            string(&mut out, "session", session);
            out.push_str(if *resumed {
                ",\"resumed\":true"
            } else {
                ",\"resumed\":false"
            });
            number(&mut out, "heartbeat_seconds", u64::from(*heartbeat_seconds));
        }
        WireFrame::Ping { nonce } => {
            tag(&mut out, "ping");
            number(&mut out, "nonce", *nonce);
        }
        WireFrame::Pong { nonce } => {
            tag(&mut out, "pong");
            number(&mut out, "nonce", *nonce);
        }
        WireFrame::Subscribe {
            stream,
            from,
            credit,
        } => {
            tag(&mut out, "subscribe");
            string(&mut out, "stream", stream);
            number(&mut out, "from", *from);
            number(&mut out, "credit", u64::from(*credit));
        }
        WireFrame::Unsubscribe { stream } => {
            tag(&mut out, "unsubscribe");
            string(&mut out, "stream", stream);
        }
        WireFrame::Credit { stream, additional } => {
            tag(&mut out, "credit");
            string(&mut out, "stream", stream);
            number(&mut out, "additional", u64::from(*additional));
        }
        WireFrame::Event {
            stream,
            sequence,
            payload,
        } => {
            tag(&mut out, "event");
            string(&mut out, "stream", stream);
            number(&mut out, "sequence", *sequence);
            payload_field(&mut out, payload);
        }
        WireFrame::Lagged { stream, skipped_to } => {
            tag(&mut out, "lagged");
            string(&mut out, "stream", stream);
            number(&mut out, "skipped_to", *skipped_to);
        }
        WireFrame::Command { id, verb, payload } => {
            tag(&mut out, "command");
            number(&mut out, "id", *id);
            string(&mut out, "verb", verb);
            payload_field(&mut out, payload);
        }
        WireFrame::Ack { id, payload } => {
            tag(&mut out, "ack");
            number(&mut out, "id", *id);
            payload_field(&mut out, payload);
        }
        WireFrame::Error { id, code, message } => {
            tag(&mut out, "error");
            match id {
                Some(id) => number(&mut out, "id", *id),
                None => out.push_str(",\"id\":null"),
            }
            string(&mut out, "code", code);
            string(&mut out, "message", message);
        }
        WireFrame::Close { code, message } => {
            tag(&mut out, "close");
            string(&mut out, "code", code);
            string(&mut out, "message", message);
        }
    }
    out.push('}');
    out
}

fn tag(out: &mut String, name: &str) {
    out.push_str("\"type\":\"");
    out.push_str(name);
    out.push('"');
}

fn number(out: &mut String, key: &str, value: u64) {
    out.push_str(",\"");
    out.push_str(key);
    out.push_str("\":");
    out.push_str(&value.to_string());
}

fn string(out: &mut String, key: &str, value: &str) {
    out.push_str(",\"");
    out.push_str(key);
    out.push_str("\":");
    escape_into(out, value);
}

/// A payload that is text is written as text, so a build log is readable in a devtools frame
/// inspector. Anything else is base64url under a key the decoder treats differently, so the two
/// can never be confused for one another.
fn payload_field(out: &mut String, payload: &[u8]) {
    match std::str::from_utf8(payload) {
        Ok(text) => {
            out.push_str(",\"payload\":");
            escape_into(out, text);
        }
        Err(_) => {
            out.push_str(",\"payload_b64\":\"");
            out.push_str(&base64url(payload));
            out.push('"');
        }
    }
}

fn escape_into(out: &mut String, value: &str) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            // `<` and `/` are escaped so a frame can never close a `<script>` element if one is
            // ever embedded in a page. It costs two bytes and removes a whole class of mistake.
            '<' => out.push_str("\\u003c"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

const B64URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// base64url without padding — the encoding used for a non-UTF-8 payload.
#[must_use]
pub fn base64url(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = chunk.get(1).copied().map_or(0, u32::from);
        let b2 = chunk.get(2).copied().map_or(0, u32::from);
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64URL[(triple >> 18) as usize & 63] as char);
        out.push(B64URL[(triple >> 12) as usize & 63] as char);
        if chunk.len() > 1 {
            out.push(B64URL[(triple >> 6) as usize & 63] as char);
        }
        if chunk.len() > 2 {
            out.push(B64URL[triple as usize & 63] as char);
        }
    }
    out
}

/// Inverse of [`base64url`]. `None` for anything that is not exactly that encoding — padding
/// included, because accepting two spellings of the same bytes is how a length check gets skipped.
#[must_use]
pub fn base64url_decode(input: &str) -> Option<Vec<u8>> {
    let bytes = input.as_bytes();
    if bytes.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    let mut accumulator: u32 = 0;
    let mut bits = 0u32;
    for byte in bytes {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        };
        accumulator = (accumulator << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((accumulator >> bits) as u8);
        }
    }
    // Leftover bits must be zero; otherwise two different inputs decode to the same bytes.
    if accumulator & ((1 << bits) - 1) != 0 {
        return None;
    }
    Some(out)
}

// ---------------------------------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------------------------------

/// One field's value. Frames contain no arrays and no nested objects, so this is the whole set.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Scalar {
    Text(String),
    /// Kept as the source text and parsed on demand, so a `u64` sequence never passes through a
    /// float and loses its low bits.
    Number(String),
    Bool(bool),
    Null,
}

/// A frame's fields, in the order they were sent.
type Fields = Vec<(String, Scalar)>;

/// Parse one JSON text frame.
///
/// # Errors
/// [`WireError`], with no more detail than the peer needs.
pub fn decode(text: &str) -> Result<WireFrame, WireError> {
    if text.len() > MAX_FRAME_BYTES {
        return Err(WireError::TooLarge);
    }
    let fields = parse_flat_object(text)?;
    let kind = text_field(&fields, "type")?;
    Ok(match kind {
        "hello" => WireFrame::Hello {
            version: u16::try_from(number_field(&fields, "version")?)
                .map_err(|_| WireError::BadField)?,
            resume: optional_text_field(&fields, "resume")?.map(str::to_owned),
        },
        "welcome" => WireFrame::Welcome {
            session: text_field(&fields, "session")?.to_owned(),
            resumed: bool_field(&fields, "resumed")?,
            heartbeat_seconds: u32_field(&fields, "heartbeat_seconds")?,
        },
        "ping" => WireFrame::Ping {
            nonce: number_field(&fields, "nonce")?,
        },
        "pong" => WireFrame::Pong {
            nonce: number_field(&fields, "nonce")?,
        },
        "subscribe" => WireFrame::Subscribe {
            stream: text_field(&fields, "stream")?.to_owned(),
            from: number_field(&fields, "from")?,
            credit: u32_field(&fields, "credit")?,
        },
        "unsubscribe" => WireFrame::Unsubscribe {
            stream: text_field(&fields, "stream")?.to_owned(),
        },
        "credit" => WireFrame::Credit {
            stream: text_field(&fields, "stream")?.to_owned(),
            additional: u32_field(&fields, "additional")?,
        },
        "event" => WireFrame::Event {
            stream: text_field(&fields, "stream")?.to_owned(),
            sequence: number_field(&fields, "sequence")?,
            payload: payload_of(&fields)?,
        },
        "lagged" => WireFrame::Lagged {
            stream: text_field(&fields, "stream")?.to_owned(),
            skipped_to: number_field(&fields, "skipped_to")?,
        },
        "command" => WireFrame::Command {
            id: number_field(&fields, "id")?,
            verb: text_field(&fields, "verb")?.to_owned(),
            payload: payload_of(&fields)?,
        },
        "ack" => WireFrame::Ack {
            id: number_field(&fields, "id")?,
            payload: payload_of(&fields)?,
        },
        "error" => WireFrame::Error {
            id: match fields
                .iter()
                .find(|(key, _)| key == "id")
                .map(|(_, value)| value)
            {
                None | Some(Scalar::Null) => None,
                Some(Scalar::Number(raw)) => Some(raw.parse().map_err(|_| WireError::BadField)?),
                Some(_) => return Err(WireError::BadField),
            },
            code: text_field(&fields, "code")?.to_owned(),
            message: text_field(&fields, "message")?.to_owned(),
        },
        "close" => WireFrame::Close {
            code: text_field(&fields, "code")?.to_owned(),
            message: text_field(&fields, "message")?.to_owned(),
        },
        _ => return Err(WireError::UnknownType),
    })
}

fn find<'a>(fields: &'a [(String, Scalar)], key: &str) -> Option<&'a Scalar> {
    fields
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value)
}

fn text_field<'a>(fields: &'a [(String, Scalar)], key: &str) -> Result<&'a str, WireError> {
    match find(fields, key) {
        Some(Scalar::Text(value)) => Ok(value),
        _ => Err(WireError::BadField),
    }
}

fn optional_text_field<'a>(
    fields: &'a [(String, Scalar)],
    key: &str,
) -> Result<Option<&'a str>, WireError> {
    match find(fields, key) {
        None | Some(Scalar::Null) => Ok(None),
        Some(Scalar::Text(value)) => Ok(Some(value)),
        Some(_) => Err(WireError::BadField),
    }
}

fn number_field(fields: &[(String, Scalar)], key: &str) -> Result<u64, WireError> {
    match find(fields, key) {
        Some(Scalar::Number(raw)) => raw.parse().map_err(|_| WireError::BadField),
        _ => Err(WireError::BadField),
    }
}

fn u32_field(fields: &[(String, Scalar)], key: &str) -> Result<u32, WireError> {
    u32::try_from(number_field(fields, key)?).map_err(|_| WireError::BadField)
}

fn bool_field(fields: &[(String, Scalar)], key: &str) -> Result<bool, WireError> {
    match find(fields, key) {
        Some(Scalar::Bool(value)) => Ok(*value),
        _ => Err(WireError::BadField),
    }
}

/// A payload is text or base64url, never both and never neither.
fn payload_of(fields: &[(String, Scalar)]) -> Result<Vec<u8>, WireError> {
    let text = find(fields, "payload");
    let encoded = find(fields, "payload_b64");
    match (text, encoded) {
        (Some(Scalar::Text(value)), None) => Ok(value.as_bytes().to_vec()),
        (None, Some(Scalar::Text(value))) => base64url_decode(value).ok_or(WireError::BadField),
        (None, None) => Ok(Vec::new()),
        _ => Err(WireError::BadField),
    }
}

/// Parse `{"a":1,"b":"two"}` and nothing more ambitious.
fn parse_flat_object(text: &str) -> Result<Fields, WireError> {
    let bytes = text.as_bytes();
    let mut at = skip_whitespace(bytes, 0);
    if bytes.get(at) != Some(&b'{') {
        return Err(WireError::NotAFrame);
    }
    at += 1;
    let mut fields: Fields = Vec::new();
    at = skip_whitespace(bytes, at);
    if bytes.get(at) == Some(&b'}') {
        return finish(bytes, at + 1, fields);
    }
    loop {
        at = skip_whitespace(bytes, at);
        let (key, next) = parse_string(bytes, at)?;
        at = skip_whitespace(bytes, next);
        if bytes.get(at) != Some(&b':') {
            return Err(WireError::NotAFrame);
        }
        at = skip_whitespace(bytes, at + 1);
        let (value, next) = parse_scalar(bytes, at)?;
        at = skip_whitespace(bytes, next);
        if fields.len() >= MAX_KEYS {
            return Err(WireError::TooLarge);
        }
        // A key sent twice is refused rather than resolved: "last one wins" is how two parsers
        // read the same frame differently.
        if fields.iter().any(|(existing, _)| *existing == key) {
            return Err(WireError::NotAFrame);
        }
        fields.push((key, value));
        match bytes.get(at) {
            Some(b',') => at += 1,
            Some(b'}') => return finish(bytes, at + 1, fields),
            _ => return Err(WireError::NotAFrame),
        }
    }
}

fn finish(bytes: &[u8], at: usize, fields: Fields) -> Result<Fields, WireError> {
    if skip_whitespace(bytes, at) == bytes.len() {
        Ok(fields)
    } else {
        Err(WireError::NotAFrame)
    }
}

const fn skip_whitespace(bytes: &[u8], mut at: usize) -> usize {
    while at < bytes.len() && matches!(bytes[at], b' ' | b'\t' | b'\n' | b'\r') {
        at += 1;
    }
    at
}

fn parse_scalar(bytes: &[u8], at: usize) -> Result<(Scalar, usize), WireError> {
    match bytes.get(at) {
        Some(b'"') => parse_string(bytes, at).map(|(text, next)| (Scalar::Text(text), next)),
        Some(b't') if bytes[at..].starts_with(b"true") => Ok((Scalar::Bool(true), at + 4)),
        Some(b'f') if bytes[at..].starts_with(b"false") => Ok((Scalar::Bool(false), at + 5)),
        Some(b'n') if bytes[at..].starts_with(b"null") => Ok((Scalar::Null, at + 4)),
        Some(byte) if byte.is_ascii_digit() => {
            let mut end = at;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            // 20 digits is already past u64::MAX; a longer run is not a number we will use.
            if end - at > 20 {
                return Err(WireError::TooLarge);
            }
            // A leading zero would let `01` and `1` be the same value; refuse the ambiguity.
            if bytes[at] == b'0' && end - at > 1 {
                return Err(WireError::NotAFrame);
            }
            let raw = std::str::from_utf8(&bytes[at..end]).map_err(|_| WireError::NotAFrame)?;
            Ok((Scalar::Number(raw.to_owned()), end))
        }
        // Arrays, objects, negative numbers and floats are not part of the vocabulary.
        _ => Err(WireError::NotAFrame),
    }
}

fn parse_string(bytes: &[u8], at: usize) -> Result<(String, usize), WireError> {
    if bytes.get(at) != Some(&b'"') {
        return Err(WireError::NotAFrame);
    }
    let mut out = String::new();
    let mut index = at + 1;
    loop {
        let byte = *bytes.get(index).ok_or(WireError::NotAFrame)?;
        if out.len() > MAX_STRING_BYTES {
            return Err(WireError::TooLarge);
        }
        match byte {
            b'"' => return Ok((out, index + 1)),
            b'\\' => {
                let escape = *bytes.get(index + 1).ok_or(WireError::NotAFrame)?;
                index += 2;
                match escape {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'b' => out.push('\u{8}'),
                    b'f' => out.push('\u{c}'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'u' => {
                        let (ch, next) = parse_unicode_escape(bytes, index)?;
                        out.push(ch);
                        index = next;
                    }
                    _ => return Err(WireError::NotAFrame),
                }
            }
            // A raw control character in a JSON string is invalid JSON, and accepting one is how
            // a newline gets into a log sink that treats newlines as record separators.
            0x00..=0x1f => return Err(WireError::NotAFrame),
            _ => {
                // Copy the whole UTF-8 sequence at once; `text` was a `&str`, so it is valid.
                let width = utf8_width(byte);
                let slice = bytes
                    .get(index..index + width)
                    .ok_or(WireError::NotAFrame)?;
                out.push_str(std::str::from_utf8(slice).map_err(|_| WireError::NotAFrame)?);
                index += width;
            }
        }
    }
}

const fn utf8_width(first: u8) -> usize {
    match first {
        0x00..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        _ => 4,
    }
}

/// `\uXXXX`, including a surrogate pair. `at` points at the first hex digit.
fn parse_unicode_escape(bytes: &[u8], at: usize) -> Result<(char, usize), WireError> {
    let high = hex4(bytes, at)?;
    let mut next = at + 4;
    let code = if (0xd800..0xdc00).contains(&high) {
        // A high surrogate must be followed by `\uDC00..\uDFFF`; a lone one is not a character.
        if bytes.get(next) != Some(&b'\\') || bytes.get(next + 1) != Some(&b'u') {
            return Err(WireError::NotAFrame);
        }
        let low = hex4(bytes, next + 2)?;
        if !(0xdc00..0xe000).contains(&low) {
            return Err(WireError::NotAFrame);
        }
        next += 6;
        0x1_0000 + ((high - 0xd800) << 10) + (low - 0xdc00)
    } else if (0xdc00..0xe000).contains(&high) {
        return Err(WireError::NotAFrame);
    } else {
        high
    };
    let ch = char::from_u32(code).ok_or(WireError::NotAFrame)?;
    Ok((ch, next))
}

fn hex4(bytes: &[u8], at: usize) -> Result<u32, WireError> {
    let digits = bytes.get(at..at + 4).ok_or(WireError::NotAFrame)?;
    let mut value = 0u32;
    for digit in digits {
        let nibble = match digit {
            b'0'..=b'9' => digit - b'0',
            b'a'..=b'f' => digit - b'a' + 10,
            b'A'..=b'F' => digit - b'A' + 10,
            _ => return Err(WireError::NotAFrame),
        };
        value = (value << 4) | u32::from(nibble);
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples() -> Vec<WireFrame> {
        vec![
            WireFrame::Hello {
                version: 1,
                resume: None,
            },
            WireFrame::Hello {
                version: 1,
                resume: Some("sess_01HZ".into()),
            },
            WireFrame::Welcome {
                session: "sess_01HZ".into(),
                resumed: true,
                heartbeat_seconds: 20,
            },
            WireFrame::Welcome {
                session: "s".into(),
                resumed: false,
                heartbeat_seconds: 0,
            },
            WireFrame::Ping { nonce: u64::MAX },
            WireFrame::Pong { nonce: 0 },
            WireFrame::Subscribe {
                stream: "run:01HZ:logs".into(),
                from: 42,
                credit: 128,
            },
            WireFrame::Unsubscribe {
                stream: "run:01HZ:logs".into(),
            },
            WireFrame::Credit {
                stream: "run:01HZ:logs".into(),
                additional: 64,
            },
            WireFrame::Event {
                stream: "run:01HZ:logs".into(),
                sequence: 1,
                payload: b"npm ci".to_vec(),
            },
            WireFrame::Event {
                stream: "s".into(),
                sequence: u64::MAX,
                payload: Vec::new(),
            },
            WireFrame::Event {
                stream: "s".into(),
                sequence: 3,
                payload: vec![0, 0xff, 0x80],
            },
            WireFrame::Lagged {
                stream: "s".into(),
                skipped_to: 900,
            },
            WireFrame::Command {
                id: 7,
                verb: "cancel".into(),
                payload: b"{}".to_vec(),
            },
            WireFrame::Ack {
                id: 7,
                payload: Vec::new(),
            },
            WireFrame::Error {
                id: Some(7),
                code: "forbidden".into(),
                message: "no".into(),
            },
            WireFrame::Error {
                id: None,
                code: "protocol".into(),
                message: "bad frame".into(),
            },
            WireFrame::Close {
                code: "going_away".into(),
                message: "deploy".into(),
            },
        ]
    }

    #[test]
    fn every_frame_round_trips() {
        for frame in samples() {
            let text = encode(&frame);
            assert_eq!(
                decode(&text),
                Ok(frame.clone()),
                "frame {frame:?} encoded as {text}"
            );
        }
    }

    #[test]
    fn a_text_payload_stays_readable_and_a_binary_one_does_not_pretend_to_be_text() {
        let text = encode(&WireFrame::Event {
            stream: "run:01HZ:logs".into(),
            sequence: 12,
            payload: b"+ cargo test --locked".to_vec(),
        });
        assert!(
            text.contains("\"payload\":\"+ cargo test --locked\""),
            "{text}"
        );
        assert!(!text.contains("payload_b64"));

        let binary = encode(&WireFrame::Event {
            stream: "s".into(),
            sequence: 1,
            payload: vec![0xff, 0xfe],
        });
        assert!(binary.contains("\"payload_b64\":"), "{binary}");
        assert!(!binary.contains("\"payload\":"));
    }

    #[test]
    fn control_characters_and_script_ends_are_escaped_on_the_way_out() {
        let text = encode(&WireFrame::Event {
            stream: "s".into(),
            sequence: 1,
            payload: b"line one\nline\ttwo </script> \"quoted\" \\ back".to_vec(),
        });
        assert!(
            !text.contains('\n'),
            "a raw newline would split the frame: {text}"
        );
        assert!(!text.contains("</script>"), "{text}");
        assert!(text.contains("\\u003c/script>"), "{text}");
        // And it survives the round trip unchanged.
        let WireFrame::Event { payload, .. } = decode(&text).unwrap() else {
            panic!("not an event")
        };
        assert_eq!(payload, b"line one\nline\ttwo </script> \"quoted\" \\ back");
    }

    #[test]
    fn junk_is_refused_rather_than_guessed_at() {
        for (input, expected) in [
            ("", WireError::NotAFrame),
            ("null", WireError::NotAFrame),
            ("[]", WireError::NotAFrame),
            ("{\"type\":\"ping\"}", WireError::BadField),
            ("{\"type\":\"nope\",\"x\":1}", WireError::UnknownType),
            ("{\"type\":42}", WireError::BadField),
            ("{\"type\":\"ping\",\"nonce\":\"1\"}", WireError::BadField),
            ("{\"type\":\"ping\",\"nonce\":-1}", WireError::NotAFrame),
            ("{\"type\":\"ping\",\"nonce\":1.5}", WireError::NotAFrame),
            ("{\"type\":\"ping\",\"nonce\":01}", WireError::NotAFrame),
            ("{\"type\":\"ping\",\"nonce\":1}{}", WireError::NotAFrame),
            ("{\"type\":\"ping\",\"nonce\":1,}", WireError::NotAFrame),
            ("{\"type\":\"ping\",\"nonce\":1,\"nonce\":2}", WireError::NotAFrame),
            ("{\"type\":\"subscribe\",\"stream\":{\"a\":1},\"from\":0,\"credit\":1}", WireError::NotAFrame),
            ("{\"type\":\"subscribe\",\"stream\":[\"a\"],\"from\":0,\"credit\":1}", WireError::NotAFrame),
            ("{\"type\":\"hello\",\"version\":70000}", WireError::BadField),
            ("{\"type\":\"subscribe\",\"stream\":\"s\",\"from\":0,\"credit\":4294967296}", WireError::BadField),
            ("{\"type\":\"event\",\"stream\":\"s\",\"sequence\":1,\"payload\":\"a\",\"payload_b64\":\"YQ\"}", WireError::BadField),
            ("{\"type\":\"event\",\"stream\":\"s\",\"sequence\":1,\"payload_b64\":\"!!!\"}", WireError::BadField),
        ] {
            assert_eq!(decode(input), Err(expected), "input {input}");
        }
    }

    #[test]
    fn an_oversized_frame_is_refused_before_it_is_parsed() {
        let huge = format!(
            "{{\"type\":\"ping\",\"nonce\":1,\"pad\":\"{}\"}}",
            "a".repeat(MAX_FRAME_BYTES)
        );
        assert_eq!(decode(&huge), Err(WireError::TooLarge));
        // And more keys than any frame has.
        let mut many = String::from("{\"type\":\"ping\"");
        for index in 0..MAX_KEYS + 2 {
            many.push_str(&format!(",\"k{index}\":1"));
        }
        many.push('}');
        assert_eq!(decode(&many), Err(WireError::TooLarge));
    }

    #[test]
    fn escapes_decode_including_surrogate_pairs() {
        let frame = decode(
            "{\"type\":\"event\",\"stream\":\"s\",\"sequence\":1,\"payload\":\"a\\u003cb \\ud83d\\ude80 \\t\\\"\\\\\\/\"}",
        )
        .unwrap();
        let WireFrame::Event { payload, .. } = frame else {
            panic!("not an event")
        };
        assert_eq!(String::from_utf8(payload).unwrap(), "a<b 🚀 \t\"\\/");

        for bad in [
            "{\"type\":\"event\",\"stream\":\"s\",\"sequence\":1,\"payload\":\"\\ud83d\"}",
            "{\"type\":\"event\",\"stream\":\"s\",\"sequence\":1,\"payload\":\"\\udc00\"}",
            "{\"type\":\"event\",\"stream\":\"s\",\"sequence\":1,\"payload\":\"\\uzzzz\"}",
            "{\"type\":\"event\",\"stream\":\"s\",\"sequence\":1,\"payload\":\"\\q\"}",
        ] {
            assert_eq!(decode(bad), Err(WireError::NotAFrame), "input {bad}");
        }
    }

    #[test]
    fn a_raw_control_character_inside_a_string_is_not_accepted() {
        let raw = "{\"type\":\"event\",\"stream\":\"s\",\"sequence\":1,\"payload\":\"a\nb\"}";
        assert_eq!(decode(raw), Err(WireError::NotAFrame));
    }

    #[test]
    fn base64url_round_trips_and_refuses_a_second_spelling() {
        for bytes in [
            vec![],
            vec![0],
            vec![0, 1],
            vec![0, 1, 2],
            (0..=255u8).collect::<Vec<u8>>(),
        ] {
            let encoded = base64url(&bytes);
            assert!(
                !encoded.contains('='),
                "padding is not part of the encoding: {encoded}"
            );
            assert_eq!(base64url_decode(&encoded), Some(bytes));
        }
        assert_eq!(base64url_decode("YQ=="), None, "padding is refused");
        assert_eq!(
            base64url_decode("Y+8="),
            None,
            "the standard alphabet is refused"
        );
        assert_eq!(
            base64url_decode("Y"),
            None,
            "a one-character group is not a byte"
        );
        // `YR` and `YS` would both decode to the same first byte if the trailing bits were ignored.
        assert_eq!(base64url_decode("YQ"), Some(vec![0x61]));
        assert_eq!(base64url_decode("YR"), None);
    }

    #[test]
    fn whitespace_between_tokens_is_tolerated_but_trailing_junk_is_not() {
        assert_eq!(
            decode("  { \"type\" : \"ping\" , \"nonce\" : 9 }  "),
            Ok(WireFrame::Ping { nonce: 9 })
        );
        assert_eq!(
            decode("{\"type\":\"ping\",\"nonce\":9} x"),
            Err(WireError::NotAFrame)
        );
    }
}
