#![forbid(unsafe_code)]

use crate::config::WebConfig;
use crate::pages;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

const MAX_REQUEST_BYTES: usize = 8 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(5);

pub fn run(config: &WebConfig) {
    let listener = TcpListener::bind(&config.bind)
        .unwrap_or_else(|error| panic!("failed to bind web listener at {}: {error}", config.bind));
    eprintln!("gha-indie-worker web listening on {}", config.bind);

    for connection in listener.incoming() {
        match connection {
            Ok(mut stream) => {
                if let Err(error) = handle_connection(&mut stream) {
                    eprintln!("web connection failed: {error}");
                }
            }
            Err(error) => eprintln!("web accept failed: {error}"),
        }
    }
}

fn handle_connection(stream: &mut TcpStream) -> std::io::Result<()> {
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;

    let mut request = [0_u8; MAX_REQUEST_BYTES];
    let size = stream.read(&mut request)?;
    if size == 0 {
        return Ok(());
    }

    let request = String::from_utf8_lossy(&request[..size]);
    let request_line = request.lines().next().unwrap_or_default();
    let (status, content_type, body) = response_for(request_line);
    write_response(stream, status, content_type, body.as_bytes())
}

fn response_for(request_line: &str) -> (&'static str, &'static str, String) {
    match request_line {
        line if line.starts_with("GET /healthz ") || line.starts_with("GET /health ") => (
            "200 OK",
            "application/json",
            r#"{"ok":true,"service":"gha-indie-worker-web-server"}"#.to_owned(),
        ),
        line if line.starts_with("GET / ") => (
            "200 OK",
            "text/html; charset=utf-8",
            pages::home::markup(),
        ),
        line if line.starts_with("GET ") => (
            "404 Not Found",
            "text/plain; charset=utf-8",
            "not found\n".to_owned(),
        ),
        _ => (
            "405 Method Not Allowed",
            "text/plain; charset=utf-8",
            "method not allowed\n".to_owned(),
        ),
    }
}

fn write_response(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
) -> std::io::Result<()> {
    let headers = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",
        body.len()
    );
    stream.write_all(headers.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_renders_the_web_surface() {
        let (status, content_type, body) = response_for("GET / HTTP/1.1");
        assert_eq!(status, "200 OK");
        assert_eq!(content_type, "text/html; charset=utf-8");
        assert!(body.contains("GHA Indie Worker"));
    }

    #[test]
    fn health_endpoint_is_available_to_compose() {
        let (status, content_type, body) = response_for("GET /healthz HTTP/1.1");
        assert_eq!(status, "200 OK");
        assert_eq!(content_type, "application/json");
        assert!(body.contains("\"ok\":true"));
    }
}
