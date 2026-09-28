#![forbid(unsafe_code)]

use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    time::Duration,
};

use crate::{config::WebConfig, error::WebError, pages};

const MAX_REQUEST_HEAD_BYTES: usize = 16 * 1024;
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(5);

pub fn run(config: &WebConfig) -> Result<(), WebError> {
    let address = loopback_socket(&config.bind)?;
    let listener = TcpListener::bind(address)?;
    eprintln!("gha-indie-worker-web-server listening on http://{address}");

    for connection in listener.incoming() {
        match connection {
            Ok(mut stream) => {
                if let Err(error) = handle_connection(&mut stream, config) {
                    eprintln!("gha-indie-worker-web-server connection error: {error}");
                }
            }
            Err(error) => eprintln!("gha-indie-worker-web-server accept error: {error}"),
        }
    }

    Ok(())
}

fn loopback_socket(endpoint: &str) -> Result<SocketAddr, WebError> {
    let address = endpoint
        .trim()
        .parse::<SocketAddr>()
        .map_err(|_| WebError::InvalidConfiguration("GHA_INDIE_WORKER_WEB_BIND"))?;
    if !address.ip().is_loopback() || address.port() == 0 {
        return Err(WebError::InvalidConfiguration("GHA_INDIE_WORKER_WEB_BIND"));
    }
    Ok(address)
}

fn handle_connection(stream: &mut TcpStream, config: &WebConfig) -> Result<(), WebError> {
    stream.set_read_timeout(Some(CONNECTION_TIMEOUT))?;
    stream.set_write_timeout(Some(CONNECTION_TIMEOUT))?;

    let mut bytes = [0_u8; MAX_REQUEST_HEAD_BYTES];
    let count = stream.read(&mut bytes)?;
    if count == 0 {
        return Ok(());
    }

    let head = std::str::from_utf8(&bytes[..count]).unwrap_or_default();
    let request_line = head.lines().next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();
    let path = target.split('?').next().unwrap_or(target);

    let response = route(method, path, config);
    stream.write_all(&response)?;
    stream.flush()?;
    Ok(())
}

fn route(method: &str, path: &str, config: &WebConfig) -> Vec<u8> {
    match (method, path) {
        ("GET", "/") => html_response(200, &pages::home::markup(config.api_http_base.as_deref())),
        ("GET", "/healthz") | ("GET", "/readyz") => {
            html_response(200, &pages::health::markup())
        }
        ("HEAD", "/healthz") | ("HEAD", "/readyz") => raw_response(200, "text/html; charset=utf-8", &[]),
        ("GET" | "HEAD", _) => html_response(404, "<h1>Not found</h1>"),
        _ => html_response(405, "<h1>Method not allowed</h1>"),
    }
}

fn html_response(status: u16, body: &str) -> Vec<u8> {
    raw_response(status, "text/html; charset=utf-8", body.as_bytes())
}

fn raw_response(status: u16, content_type: &str, body: &[u8]) -> Vec<u8> {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Error",
    };
    let mut response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    response.extend_from_slice(body);
    response
}

#[cfg(test)]
mod tests {
    use super::{loopback_socket, route};
    use crate::config::WebConfig;

    fn config() -> WebConfig {
        WebConfig {
            bind: "127.0.0.1:18091".into(),
            api_http_base: Some("http://127.0.0.1:18090".into()),
            database_url: None,
        }
    }

    #[test]
    fn listener_must_be_literal_loopback() {
        assert!(loopback_socket("127.0.0.1:18091").is_ok());
        assert!(loopback_socket("[::1]:18091").is_ok());
        assert!(loopback_socket("0.0.0.0:18091").is_err());
    }

    #[test]
    fn home_and_health_are_real_http_responses() {
        let home = String::from_utf8(route("GET", "/", &config())).expect("utf8 home");
        assert!(home.starts_with("HTTP/1.1 200 OK"));
        assert!(home.contains("GHA Indie Worker"));

        let ready = String::from_utf8(route("GET", "/readyz", &config())).expect("utf8 ready");
        assert!(ready.contains("web health ok"));
    }
}
