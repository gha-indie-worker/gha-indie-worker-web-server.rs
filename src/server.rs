#![forbid(unsafe_code)]
#![allow(clippy::needless_return)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};

use crate::config::WebConfig;
use crate::pages;

fn response_for_path(path: &str) -> (&'static str, &'static str, String) {
    if path == "/readyz" {
        return (
            "200 OK",
            "application/json",
            "{\"ok\":true,\"service\":\"gha-indie-worker-web-server\"}".to_owned(),
        );
    }

    if path == "/" {
        return (
            "200 OK",
            "text/html; charset=utf-8",
            pages::home::markup(),
        );
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
    let (status, content_type, body) = response_for_path(path);
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
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
        handle_http_connection(&mut stream)?;
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
