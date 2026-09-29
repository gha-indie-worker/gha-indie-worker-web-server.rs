#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};

use crate::config::WebConfig;
use crate::error::WebError;
use crate::pages;

const MAX_REQUEST_HEAD: usize = 16 * 1024;

fn response_for(method: &str, path: &str) -> (u16, &'static str, String) {
    if method != "GET" {
        return (
            405,
            "text/plain; charset=utf-8",
            "method not allowed".to_owned(),
        );
    }

    match path {
        "/healthz" | "/readyz" => (
            200,
            "text/html; charset=utf-8",
            pages::health::markup(),
        ),
        "/" => (200, "text/html; charset=utf-8", pages::home::markup()),
        _ => (404, "text/plain; charset=utf-8", "not found".to_owned()),
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Internal Server Error",
    }
}

fn handle_http(mut stream: TcpStream) -> Result<(), WebError> {
    stream.set_read_timeout(Some(std::time::Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(std::time::Duration::from_secs(3)))?;

    let mut buffer = [0_u8; MAX_REQUEST_HEAD];
    let read = stream.read(&mut buffer)?;
    if read == 0 {
        return Ok(());
    }
    let request = std::str::from_utf8(&buffer[..read]).unwrap_or_default();
    let mut parts = request.lines().next().unwrap_or_default().split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or("/");
    let path = target.split('?').next().unwrap_or(target);

    let (status, content_type, body) = response_for(method, path);
    let head = format!(
        "HTTP/1.1 {status} {}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\ncache-control: no-store\r\nconnection: close\r\n\r\n",
        reason(status),
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body.as_bytes())?;
    stream.flush()?;
    Ok(())
}

pub fn run(config: &WebConfig) -> Result<(), WebError> {
    if config.bind.trim().is_empty() {
        return Err(WebError::Unavailable);
    }
    eprintln!("web bind {}", config.bind);
    let listener = TcpListener::bind(config.bind.trim())?;
    for connection in listener.incoming() {
        match connection {
            Ok(stream) => {
                if let Err(error) = handle_http(stream) {
                    eprintln!("web http connection failed: {error}");
                }
            }
            Err(error) => eprintln!("web http accept failed: {error}"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::response_for;

    #[test]
    fn health_and_readiness_are_real_http_routes() {
        for path in ["/healthz", "/readyz"] {
            let (status, content_type, body) = response_for("GET", path);
            assert_eq!(status, 200);
            assert_eq!(content_type, "text/html; charset=utf-8");
            assert!(body.contains("health ok"));
        }
    }

    #[test]
    fn home_is_served() {
        let (status, _, body) = response_for("GET", "/");
        assert_eq!(status, 200);
        assert!(body.contains("GHA Indie Worker"));
    }

    #[test]
    fn unknown_routes_fail_closed() {
        let (status, _, _) = response_for("GET", "/not-a-route");
        assert_eq!(status, 404);
    }
}
