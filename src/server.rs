#![forbid(unsafe_code)]
#![allow(clippy::needless_return)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};

use crate::config::WebConfig;
use crate::pages;

fn response_for_path(path: &str) -> (&'static str, &'static str, String) {
    if matches!(path, "/readyz" | "/health" | "/healthz") {
        return (
            "200 OK",
            "application/json",
            "{\"ok\":true,\"service\":\"gha-indie-worker-web-server\"}".to_owned(),
        );
    }

    if path == "/api/runs" {
        let evidence = std::env::var_os("GHA_INDIE_WORKER_STATUS_FILE")
            .and_then(|path| {
                let file = std::fs::File::open(path).ok()?;
                let mut body = String::new();
                file.take(1024 * 1024 + 1).read_to_string(&mut body).ok()?;
                Some(body)
            })
            .filter(|body| body.len() <= 1024 * 1024)
            .filter(|body| serde_json::from_str::<serde_json::Value>(body).is_ok());
        return match evidence {
            Some(body) => ("200 OK", "application/json", body),
            None => (
                "503 Service Unavailable",
                "application/json",
                "{\"error\":\"execution evidence unavailable\"}".into(),
            ),
        };
    }

    if path == "/" {
        return ("200 OK", "text/html; charset=utf-8", pages::home::markup());
    }

    return (
        "404 Not Found",
        "text/plain; charset=utf-8",
        "not found\n".to_owned(),
    );
}

fn handle_http_connection(stream: &mut TcpStream) -> std::io::Result<()> {
    let mut buffer = [0_u8; 8192];
    let bytes_read = stream.read(&mut buffer)?;
    if bytes_read == 0 {
        return Ok(());
    }

    let request = String::from_utf8_lossy(&buffer[..bytes_read]);
    let path = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");
    let method = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().next());
    let (status, content_type, body) = if matches!(method, Some("GET" | "HEAD")) {
        response_for_path(path)
    } else {
        (
            "405 Method Not Allowed",
            "text/plain",
            "read-only service\n".into(),
        )
    };
    let payload = if method == Some("HEAD") {
        ""
    } else {
        body.as_str()
    };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n{payload}",
        body.len()
    );

    stream.write_all(response.as_bytes())?;
    stream.flush()?;
    return Ok(());
}

pub fn run(config: &WebConfig) -> std::io::Result<()> {
    let listener = TcpListener::bind(config.bind.trim())?;
    println!("web Http endpoint {}", config.bind.trim());

    for connection in listener.incoming() {
        let mut stream = connection?;
        stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(std::time::Duration::from_secs(5)))?;
        // A disconnected or slow client must not terminate the server.
        let _ = handle_http_connection(&mut stream);
    }

    return Ok(());
}

#[cfg(test)]
mod tests {
    use super::response_for_path;

    #[test]
    fn readiness_and_home_are_served_and_unknown_paths_fail_closed() {
        let (status, content_type, body) = response_for_path("/readyz");
        assert_eq!(status, "200 OK");
        assert_eq!(content_type, "application/json");
        assert!(body.contains("\"ok\":true"));

        let (status, content_type, body) = response_for_path("/");
        assert_eq!(status, "200 OK");
        assert_eq!(content_type, "text/html; charset=utf-8");
        assert!(!body.is_empty());

        let (status, _, body) = response_for_path("/missing");
        assert_eq!(status, "404 Not Found");
        assert_eq!(body, "not found\n");
    }
}
