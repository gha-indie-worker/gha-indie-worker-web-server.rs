#![forbid(unsafe_code)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;

use crate::config::WebConfig;
use crate::pages;

/// Bounds on what one client may make this process read before it has proven
/// itself to be an HTTP request at all.
const MAX_REQUEST_LINE_BYTES: u64 = 8 * 1024;
const MAX_HEADER_BYTES: u64 = 32 * 1024;
const MAX_HEADER_LINES: usize = 100;

/// Serve the web surface until the process is stopped.
///
/// `ores-compose` starts this service, polls `/healthz`, and treats an exit
/// before the first successful probe as a failed start, so this function must
/// not return while the listener is healthy.
pub fn run(config: &WebConfig) {
    let listener = match TcpListener::bind(&config.bind) {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("web bind {} failed: {error}", config.bind);
            std::process::exit(1);
        }
    };

    println!("web bind {}", config.bind);
    if let Some(base) = config.api_http_base.as_deref() {
        println!("web api base {base}");
    }

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                thread::spawn(move || {
                    if let Err(error) = serve(stream) {
                        eprintln!("web connection ended: {error}");
                    }
                });
            }
            Err(error) => eprintln!("web accept failed: {error}"),
        }
    }
}

fn serve(stream: TcpStream) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    if (&mut reader)
        .take(MAX_REQUEST_LINE_BYTES)
        .read_line(&mut request_line)?
        == 0
    {
        return Ok(());
    }

    let mut header_reader = (&mut reader).take(MAX_HEADER_BYTES);
    for _ in 0..MAX_HEADER_LINES {
        let mut header = String::new();
        if header_reader.read_line(&mut header)? == 0 || header.trim().is_empty() {
            break;
        }
    }

    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();
    let path = target.split('?').next().unwrap_or_default();

    let (status, content_type, body) = match (method, path) {
        ("GET" | "HEAD", "/healthz" | "/readyz") => (
            "200 OK",
            "application/json",
            r#"{"ok":true,"service":"gha-indie-worker-web-server"}"#.to_string(),
        ),
        ("GET" | "HEAD", "/health") => (
            "200 OK",
            "text/html; charset=utf-8",
            pages::health::markup(),
        ),
        ("GET" | "HEAD", "/") => ("200 OK", "text/html; charset=utf-8", pages::home::markup()),
        ("GET" | "HEAD", _) => (
            "404 Not Found",
            "application/json",
            r#"{"error":"not_found"}"#.to_string(),
        ),
        _ => (
            "405 Method Not Allowed",
            "application/json",
            r#"{"error":"method_not_allowed"}"#.to_string(),
        ),
    };

    let mut stream = stream;
    write!(
        stream,
        "HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    )?;
    if method != "HEAD" {
        stream.write_all(body.as_bytes())?;
    }
    stream.flush()
}
