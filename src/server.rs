#![forbid(unsafe_code)]

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::config::WebConfig;
use crate::pages;

// A thread-per-client implementation is safe only with an explicit admission
// budget and deadlines. Exhausted slots never queue additional worker threads.
const MAX_CONCURRENT_CLIENTS: usize = 32;
const READ_TIMEOUT: Duration = Duration::from_secs(3);
const WRITE_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_REQUEST_LINE_BYTES: usize = 8 * 1024;
const MAX_HEADER_BYTES: usize = 32 * 1024;
const MAX_HEADER_LINES: usize = 100;

struct ConnectionPermit(Arc<AtomicUsize>);

impl Drop for ConnectionPermit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

fn try_admit(active: &Arc<AtomicUsize>) -> Option<ConnectionPermit> {
    active
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
            (count < MAX_CONCURRENT_CLIENTS).then_some(count + 1)
        })
        .ok()
        .map(|_| ConnectionPermit(Arc::clone(active)))
}

/// Run a bounded HTTP surface until the process is stopped.
///
/// This is a minimal health/UI server, not a general HTTP proxy: each accepted
/// socket has read/write deadlines, bounded header parsing and a connection
/// admission permit that is returned on every thread exit, including panics.
pub fn run(config: &WebConfig) -> io::Result<()> {
    // Preserve mainline's typed startup/error contract: never terminate the
    // whole process from a library function when binding fails.
    let listener = TcpListener::bind(config.bind.trim())?;

    println!("web bind {}", config.bind);
    let active = Arc::new(AtomicUsize::new(0));
    for connection in listener.incoming() {
        // Accept failures are still propagated to the runtime operator. Bad
        // *client requests* receive bounded 4xx responses inside serve().
        dispatch_connection(connection?, &active);
    }
    Ok(())
}

// Keep overload handling independently testable using a real TCP peer. All
// slots are accounted before spawning and returned even on spawn failure.
fn dispatch_connection(mut stream: TcpStream, active: &Arc<AtomicUsize>) {
    if let Some(permit) = try_admit(active) {
        if let Err(error) = thread::Builder::new()
            .name("giw-web-client".into())
            .spawn(move || {
                let _permit = permit;
                if let Err(error) = serve(stream) {
                    eprintln!("web connection ended: {error}");
                }
            })
        {
            eprintln!("web client thread admission failed: {error}");
        }
    } else {
        let _ = stream.set_write_timeout(Some(Duration::from_millis(250)));
        let _ = write_response(
            &mut stream,
            "503 Service Unavailable",
            "application/json",
            r#"{"error":"at_capacity"}"#,
            false,
        );
    }
}

// Enforce a total request deadline, not merely an idle socket timeout:
// a slow-drip client must not hold one of the bounded permits indefinitely.
struct DeadlineRead {
    stream: TcpStream,
    until: Instant,
}

impl Read for DeadlineRead {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let remaining = self
            .until
            .checked_duration_since(Instant::now())
            .filter(|duration| !duration.is_zero())
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "request deadline expired"))?;
        self.stream.set_read_timeout(Some(remaining))?;
        self.stream.read(out)
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum IntakeError {
    Malformed,
    TooLarge,
    TimedOut,
}

impl IntakeError {
    fn from_io(error: io::Error) -> Self {
        match error.kind() {
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => Self::TimedOut,
            _ => Self::Malformed,
        }
    }

    fn response(self) -> (&'static str, &'static str) {
        match self {
            Self::Malformed => ("400 Bad Request", r#"{"error":"bad_request"}"#),
            Self::TooLarge => ("431 Request Header Fields Too Large", r#"{"error":"headers_too_large"}"#),
            Self::TimedOut => ("408 Request Timeout", r#"{"error":"request_timeout"}"#),
        }
    }
}

// Read at most limit+1 bytes to distinguish complete lines at the limit from
// oversized/unterminated lines. Never scan for a newline without a byte bound.
fn read_crlf_line(reader: &mut impl BufRead, limit: usize) -> Result<String, IntakeError> {
    let mut bytes = Vec::new();
    reader
        .take(limit.saturating_add(1) as u64)
        .read_until(b'\n', &mut bytes)
        .map_err(IntakeError::from_io)?;
    if bytes.len() > limit {
        return Err(IntakeError::TooLarge);
    }
    if !bytes.ends_with(b"\r\n") {
        return Err(IntakeError::Malformed);
    }
    String::from_utf8(bytes).map_err(|_| IntakeError::Malformed)
}

fn parse_request(reader: &mut impl BufRead) -> Result<(String, String), IntakeError> {
    let request_line = read_crlf_line(reader, MAX_REQUEST_LINE_BYTES)?;
    let mut parts = request_line.trim_end_matches("\r\n").split_ascii_whitespace();
    let method = parts.next().ok_or(IntakeError::Malformed)?;
    let target = parts.next().ok_or(IntakeError::Malformed)?;
    let version = parts.next().ok_or(IntakeError::Malformed)?;
    if parts.next().is_some()
        || !matches!(version, "HTTP/1.0" | "HTTP/1.1")
        || !target.starts_with('/')
        || !method.bytes().all(|b| b.is_ascii_alphabetic())
    {
        return Err(IntakeError::Malformed);
    }

    let mut header_bytes = 0usize;
    let mut complete = false;
    let mut host_seen = false;
    let mut content_length_seen = false;
    for _ in 0..MAX_HEADER_LINES {
        let line = read_crlf_line(reader, MAX_HEADER_BYTES.saturating_sub(header_bytes))?;
        header_bytes += line.len();
        if line == "\r\n" {
            complete = true;
            break;
        }
        let header = line.trim_end_matches("\r\n");
        let (name, value) = header.split_once(':').ok_or(IntakeError::Malformed)?;
        if name.is_empty()
            || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            || value.bytes().any(|b| b == 0 || b == b'\r' || b == b'\n' || b == 0x7f
                || (b < 0x20 && b != b'\t'))
        {
            return Err(IntakeError::Malformed);
        }
        if name.eq_ignore_ascii_case("host") {
            if host_seen || value.trim().is_empty() {
                return Err(IntakeError::Malformed);
            }
            host_seen = true;
        }
        // This health/UI server never reads bodies. Reject ambiguous framing
        // rather than accidentally treating a second HTTP request as headers.
        if name.eq_ignore_ascii_case("transfer-encoding") {
            return Err(IntakeError::Malformed);
        }
        if name.eq_ignore_ascii_case("content-length") {
            if content_length_seen || value.trim() != "0" {
                return Err(IntakeError::Malformed);
            }
            content_length_seen = true;
        }
    }
    if !complete {
        return Err(IntakeError::TooLarge);
    }
    if version == "HTTP/1.1" && !host_seen {
        return Err(IntakeError::Malformed);
    }
    if target.bytes().any(|b| b == 0 || b == 0x7f || b < 0x20) {
        return Err(IntakeError::Malformed);
    }

    Ok((method.to_owned(), target.split('?').next().unwrap_or_default().to_owned()))
}

fn write_response(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &str,
    head_only: bool,
) -> io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    )?;
    if !head_only {
        stream.write_all(body.as_bytes())?;
    }
    stream.flush()
}

fn serve(mut stream: TcpStream) -> io::Result<()> {
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
    let mut reader = BufReader::new(DeadlineRead {
        stream: stream.try_clone()?,
        until: Instant::now() + READ_TIMEOUT,
    });
    let (method, path) = match parse_request(&mut reader) {
        Ok(request) => request,
        Err(error) => {
            let (status, body) = error.response();
            return write_response(&mut stream, status, "application/json", body, false);
        }
    };

    let (status, content_type, body) = match (method.as_str(), path.as_str()) {
        ("GET" | "HEAD", "/healthz" | "/readyz") => (
            "200 OK",
            "application/json",
            r#"{"ok":true,"service":"gha-indie-worker-web-server"}"#.to_owned(),
        ),
        ("GET" | "HEAD", "/health") => (
            "200 OK",
            "text/html; charset=utf-8",
            pages::health::markup(),
        ),
        ("GET" | "HEAD", "/") => ("200 OK", "text/html; charset=utf-8", pages::home::markup()),
        ("GET" | "HEAD", _) => (
            "404 Not Found",
            "text/plain; charset=utf-8",
            "not found\n".to_owned(),
        ),
        _ => (
            "405 Method Not Allowed",
            "application/json",
            r#"{"error":"method_not_allowed"}"#.to_owned(),
        ),
    };
    write_response(&mut stream, status, content_type, &body, method == "HEAD")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn parse(source: &[u8]) -> Result<(String, String), IntakeError> {
        parse_request(&mut BufReader::new(Cursor::new(source)))
    }

    #[test]
    fn complete_request_admits_only_valid_route_line_and_headers() {
        assert_eq!(
            parse(b"GET /healthz?probe=1 HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap(),
            ("GET".to_owned(), "/healthz".to_owned())
        );
        assert_eq!(parse(b"GET /healthz HTTP/1.1\r\nHost: x\r\n"), Err(IntakeError::Malformed));
        assert_eq!(parse(b"GET /healthz HTTP/1.1\r\nHost: x"), Err(IntakeError::Malformed));
        assert_eq!(parse(b"GET /healthz HTTP/1.1\r\nNoColon\r\n\r\n"), Err(IntakeError::Malformed));
        assert_eq!(parse(b"GET /healthz HTTP/2.0\r\n\r\n"), Err(IntakeError::Malformed));
        assert_eq!(parse(b"GET https://example.com/ HTTP/1.1\r\n\r\n"), Err(IntakeError::Malformed));
    }

    #[test]
    fn request_line_and_headers_fail_closed_at_bounds() {
        let mut long_line = b"GET /".to_vec();
        long_line.extend(vec![b'a'; MAX_REQUEST_LINE_BYTES]);
        long_line.extend_from_slice(b" HTTP/1.1\r\n\r\n");
        assert_eq!(parse(&long_line), Err(IntakeError::TooLarge));

        let mut headers = b"GET / HTTP/1.1\r\n".to_vec();
        for _ in 0..MAX_HEADER_LINES {
            headers.extend_from_slice(b"X: y\r\n");
        }
        headers.extend_from_slice(b"\r\n");
        assert_eq!(parse(&headers), Err(IntakeError::TooLarge));

        let mut long_header = b"GET / HTTP/1.1\r\nX: ".to_vec();
        long_header.extend(vec![b'a'; MAX_HEADER_BYTES]);
        long_header.extend_from_slice(b"\r\n\r\n");
        assert_eq!(parse(&long_header), Err(IntakeError::TooLarge));
    }

    #[test]
    fn connection_budget_is_bounded_and_released() {
        let active = Arc::new(AtomicUsize::new(0));
        let permits: Vec<_> = (0..MAX_CONCURRENT_CLIENTS)
            .map(|_| try_admit(&active).expect("must admit within budget"))
            .collect();
        assert!(try_admit(&active).is_none());
        assert_eq!(active.load(Ordering::Acquire), MAX_CONCURRENT_CLIENTS);
        drop(permits);
        assert_eq!(active.load(Ordering::Acquire), 0);
        assert!(try_admit(&active).is_some());
    }

    fn socket_round_trip(source: &[u8]) -> String {
        use std::net::Shutdown;

        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback listener");
        let addr = listener.local_addr().expect("local address");
        let worker = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept loopback");
            serve(stream).expect("write response");
        });
        let mut client = TcpStream::connect(addr).expect("connect loopback");
        client
            .set_read_timeout(Some(Duration::from_secs(4)))
            .expect("read timeout");
        client.write_all(source).expect("write request");
        client.shutdown(Shutdown::Write).expect("finish request");
        let mut response = String::new();
        client.read_to_string(&mut response).expect("read response");
        worker.join().expect("worker thread");
        response
    }

    #[test]
    fn raw_socket_accepts_complete_health_and_rejects_partial_framing() {
        let success = socket_round_trip(b"GET /healthz HTTP/1.1\r\nHost: localhost\r\n\r\n");
        assert!(success.starts_with("HTTP/1.1 200 OK\r\n"), "{success}");
        assert!(success.contains("\"ok\":true"));

        let partial = socket_round_trip(b"GET /healthz HTTP/1.1\r\nHost: localhost\r\n");
        assert!(partial.starts_with("HTTP/1.1 400 Bad Request\r\n"), "{partial}");

        let invalid = socket_round_trip(b"GET /healthz HTTP/1.1\r\nWrong-Header\r\n\r\n");
        assert!(invalid.starts_with("HTTP/1.1 400 Bad Request\r\n"), "{invalid}");

        let head = socket_round_trip(b"HEAD /healthz HTTP/1.1\r\nHost: localhost\r\n\r\n");
        assert!(head.starts_with("HTTP/1.1 200 OK\r\n"), "{head}");
        assert!(head.ends_with("\r\n\r\n"), "{head}");
    }

    #[test]
    fn deadline_reader_fails_even_if_peer_keeps_connection_open() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback listener");
        let addr = listener.local_addr().expect("local address");
        let client = TcpStream::connect(addr).expect("connect");
        let (server, _) = listener.accept().expect("accept");
        let mut expired = DeadlineRead {
            stream: server,
            until: Instant::now() - Duration::from_millis(1),
        };
        let mut buf = [0u8; 1];
        assert_eq!(
            expired.read(&mut buf).unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        drop(client);
    }

}

// Keep the adversarial matrix separate from the small baseline test module.
#[cfg(test)]
#[path = "server_adversarial_tests.rs"]
mod server_adversarial_tests;
