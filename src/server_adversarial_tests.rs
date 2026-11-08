//! Adversarial tests exercise the exact HTTP intake and real loopback sockets,
//! not only acceptance of already-parsed route names.

use super::*;
use std::io::Cursor;
use std::net::Shutdown;
use std::sync::{mpsc, Barrier};

fn parse_bytes(bytes: &[u8]) -> Result<(String, String), IntakeError> {
    parse_request(&mut BufReader::new(Cursor::new(bytes)))
}

fn raw_exchange(bytes: &[u8]) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let address = listener.local_addr().expect("loopback address");
    let worker = thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept peer");
        serve(stream).expect("write bounded response");
    });
    let mut client = TcpStream::connect(address).expect("connect loopback");
    client
        .set_read_timeout(Some(Duration::from_secs(4)))
        .expect("timeout on test peer");
    client.write_all(bytes).expect("send request");
    client.shutdown(Shutdown::Write).expect("signal end of input");
    let mut response = String::new();
    client.read_to_string(&mut response).expect("receive response");
    worker.join().expect("join HTTP worker");
    response
}

#[test]
fn accepts_http_10_and_case_insensitive_host_without_implicit_bodies() {
    let cases: &[(&[u8], &str, &str)] = &[
        (b"GET / HTTP/1.0\r\n\r\n", "GET", "/"),
        (b"GET /healthz?probe=1 HTTP/1.1\r\nhOsT: localhost\r\n\r\n", "GET", "/healthz"),
        (
            b"HEAD /readyz?x=1 HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\n\r\n",
            "HEAD",
            "/readyz",
        ),
        (b"POST /healthz HTTP/1.1\r\nHost: x\r\n\r\n", "POST", "/healthz"),
    ];
    for &(source, method, path) in cases {
        let parsed = parse_bytes(source).expect("valid HTTP framing");
        assert_eq!(parsed, (method.to_owned(), path.to_owned()));
    }
}

#[test]
fn rejects_host_ambiguity_and_body_framing_that_is_never_consumed() {
    let invalid: &[(&str, &[u8])] = &[
        ("missing Host HTTP/1.1", b"GET / HTTP/1.1\r\n\r\n"),
        ("empty Host", b"GET / HTTP/1.1\r\nHost: \r\n\r\n"),
        (
            "duplicate Host",
            b"GET / HTTP/1.1\r\nHost: one\r\nhost: two\r\n\r\n",
        ),
        (
            "chunked request body",
            b"GET / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n",
        ),
        (
            "nonzero Content-Length",
            b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 1\r\n\r\n",
        ),
        (
            "duplicate Content-Length",
            b"GET / HTTP/1.1\r\nHost: x\r\nContent-Length: 0\r\ncontent-length: 0\r\n\r\n",
        ),
        (
            "misleading length zero padding",
            b"GET / HTTP/1.1\r\nHost: x\r\nContent-Length: 00\r\n\r\n",
        ),
    ];
    for &(name, source) in invalid {
        assert_eq!(parse_bytes(source), Err(IntakeError::Malformed), "{name}");
    }
}

#[test]
fn rejects_invalid_controls_in_target_and_header_values() {
    let invalid: &[(&str, &[u8])] = &[
        ("NUL in target", b"GET /health\0z HTTP/1.1\r\nHost: x\r\n\r\n"),
        ("NUL in Host", b"GET / HTTP/1.1\r\nHost: x\0y\r\n\r\n"),
        ("DEL in header", b"GET / HTTP/1.1\r\nHost: x\r\nX: a\x7fb\r\n\r\n"),
        ("bare carriage return", b"GET / HTTP/1.1\r\nHost: x\r\nX: a\rb\r\n\r\n"),
        ("control character", b"GET / HTTP/1.1\r\nHost: x\r\nX: a\x01b\r\n\r\n"),
        ("invalid UTF-8", b"GET / HTTP/1.1\r\nHost: x\r\nX: \xff\r\n\r\n"),
        ("obsolete header folding", b"GET / HTTP/1.1\r\nHost: x\r\n X: y\r\n\r\n"),
        ("bad header field name", b"GET / HTTP/1.1\r\nHost: x\r\nX bad: y\r\n\r\n"),
        ("missing header colon", b"GET / HTTP/1.1\r\nHost: x\r\nXYZ\r\n\r\n"),
        ("bad protocol", b"GET / HTTP/2.0\r\nHost: x\r\n\r\n"),
        ("absolute-form target", b"GET http://example.test/ HTTP/1.1\r\nHost: x\r\n\r\n"),
        ("extra start-line token", b"GET / HTTP/1.1 ignored\r\nHost: x\r\n\r\n"),
        ("LF-only request", b"GET / HTTP/1.1\nHost: x\n\n"),
        ("incomplete header block", b"GET / HTTP/1.1\r\nHost: x\r\n"),
    ];
    for &(name, source) in invalid {
        assert_eq!(parse_bytes(source), Err(IntakeError::Malformed), "{name}");
    }
}

#[test]
fn one_byte_null_mutation_of_valid_request_always_fails_closed() {
    let seed = b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nX-Trace: ok\r\n\r\n";
    assert!(parse_bytes(seed).is_ok());
    for i in 0..seed.len() {
        let mut mutation = seed.to_vec();
        mutation[i] = 0;
        assert_eq!(
            parse_bytes(&mutation),
            Err(IntakeError::Malformed),
            "NUL mutation at byte {i} accepted"
        );
    }
}

#[test]
fn exact_request_line_budget_admits_boundary_and_denies_one_extra_byte() {
    let fixed = b"GET /".len() + b" HTTP/1.1\r\n".len();
    let mut request = b"GET /".to_vec();
    request.extend(vec![b'a'; MAX_REQUEST_LINE_BYTES - fixed]);
    request.extend_from_slice(b" HTTP/1.1\r\nHost: x\r\n\r\n");
    assert!(parse_bytes(&request).is_ok());
    request.insert(5, b'x');
    assert_eq!(parse_bytes(&request), Err(IntakeError::TooLarge));
}

#[test]
fn exact_header_count_budget_admits_99_and_denies_100_fields() {
    let mut request = b"GET / HTTP/1.1\r\nHost: x\r\n".to_vec();
    for _ in 0..98 {
        request.extend_from_slice(b"X: y\r\n");
    }
    let mut admitted = request.clone();
    admitted.extend_from_slice(b"\r\n");
    assert!(parse_bytes(&admitted).is_ok());

    request.extend_from_slice(b"X: y\r\n\r\n");
    assert_eq!(parse_bytes(&request), Err(IntakeError::TooLarge));
}

#[test]
fn exact_header_byte_budget_denies_an_extra_byte() {
    let head = b"GET / HTTP/1.1\r\nHost: x\r\n";
    let remaining = MAX_HEADER_BYTES - b"Host: x\r\n".len() - b"X: \r\n".len() - 2;
    let mut admitted = head.to_vec();
    admitted.extend_from_slice(b"X: ");
    admitted.extend(vec![b'a'; remaining]);
    admitted.extend_from_slice(b"\r\n\r\n");
    assert!(parse_bytes(&admitted).is_ok());

    let mut exceeded = head.to_vec();
    exceeded.extend_from_slice(b"X: ");
    exceeded.extend(vec![b'a'; remaining + 1]);
    exceeded.extend_from_slice(b"\r\n\r\n");
    assert_eq!(parse_bytes(&exceeded), Err(IntakeError::TooLarge));
}

#[test]
fn raw_sockets_return_specific_failure_status_and_never_reflect_bad_input() {
    let cases: &[(&[u8], &str)] = &[
        (b"GET / HTTP/1.1\r\n\r\n", "400 Bad Request"),
        (
            b"GET / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n",
            "400 Bad Request",
        ),
        (
            b"GET / HTTP/1.1\r\nHost: x\r\nContent-Length: 3\r\n\r\n",
            "400 Bad Request",
        ),
    ];
    for &(request, expected) in cases {
        let response = raw_exchange(request);
        assert!(response.starts_with(&format!("HTTP/1.1 {expected}\r\n")), "{response}");
        assert!(response.contains("connection: close\r\n"));
        assert!(!response.contains("Transfer-Encoding: chunked"));
    }
}

#[test]
fn raw_sockets_preserve_head_length_and_route_semantics() {
    let get = raw_exchange(b"GET /healthz HTTP/1.1\r\nHost: localhost\r\n\r\n");
    let head = raw_exchange(b"HEAD /healthz HTTP/1.1\r\nHost: localhost\r\n\r\n");
    let (get_headers, get_body) = get.split_once("\r\n\r\n").expect("GET response");
    let (head_headers, head_body) = head.split_once("\r\n\r\n").expect("HEAD response");
    assert_eq!(get_headers, head_headers);
    assert!(get_body.contains("\"ok\":true"));
    assert!(head_body.is_empty());
    assert!(get_headers.contains(&format!("content-length: {}\r\n", get_body.len())));

    let missing = raw_exchange(b"GET /not-found HTTP/1.1\r\nHost: localhost\r\n\r\n");
    assert!(missing.starts_with("HTTP/1.1 404 Not Found\r\n"));
    let wrong_method = raw_exchange(b"POST /healthz HTTP/1.1\r\nHost: localhost\r\n\r\n");
    assert!(wrong_method.starts_with("HTTP/1.1 405 Method Not Allowed\r\n"));
}

#[test]
fn actual_tcp_overload_returns_503_without_spawning_worker() {
    let active = Arc::new(AtomicUsize::new(0));
    let permits: Vec<_> = (0..MAX_CONCURRENT_CLIENTS)
        .map(|_| try_admit(&active).expect("hold all worker slots"))
        .collect();
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let address = listener.local_addr().expect("address");
    let guard = Arc::clone(&active);
    let worker = thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept");
        dispatch_connection(stream, &guard);
    });
    let mut client = TcpStream::connect(address).expect("connect");
    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("read bound");
    let mut response = String::new();
    client.read_to_string(&mut response).expect("read overload");
    worker.join().expect("join");
    assert!(response.starts_with("HTTP/1.1 503 Service Unavailable\r\n"), "{response}");
    assert_eq!(active.load(Ordering::Acquire), MAX_CONCURRENT_CLIENTS);
    drop(permits);
    assert_eq!(active.load(Ordering::Acquire), 0);
}

#[test]
fn contending_threads_never_admit_more_than_the_budget() {
    const CONTENDERS: usize = MAX_CONCURRENT_CLIENTS * 2;
    let active = Arc::new(AtomicUsize::new(0));
    let start = Arc::new(Barrier::new(CONTENDERS + 1));
    let release = Arc::new(Barrier::new(CONTENDERS + 1));
    let (sender, receiver) = mpsc::channel();
    let mut workers = Vec::with_capacity(CONTENDERS);
    for _ in 0..CONTENDERS {
        let active = Arc::clone(&active);
        let start = Arc::clone(&start);
        let release = Arc::clone(&release);
        let sender = sender.clone();
        workers.push(thread::spawn(move || {
            start.wait();
            let permit = try_admit(&active);
            sender.send(permit.is_some()).expect("send admission result");
            release.wait();
            drop(permit);
        }));
    }
    drop(sender);
    start.wait();
    let mut winners = 0;
    for _ in 0..CONTENDERS {
        if receiver
            .recv_timeout(Duration::from_secs(10))
            .expect("each contender reports admission")
        {
            winners += 1;
        }
    }
    assert_eq!(winners, MAX_CONCURRENT_CLIENTS);
    assert_eq!(active.load(Ordering::Acquire), MAX_CONCURRENT_CLIENTS);
    release.wait();
    for worker in workers {
        worker.join().expect("join contender");
    }
    assert_eq!(active.load(Ordering::Acquire), 0);
}

#[test]
fn permit_is_returned_after_a_panicking_worker() {
    let active = Arc::new(AtomicUsize::new(0));
    let active_in_worker = Arc::clone(&active);
    let result = thread::spawn(move || {
        let _permit = try_admit(&active_in_worker).expect("slot");
        panic!("simulated worker failure; permit must drop");
    })
    .join();
    assert!(result.is_err());
    assert_eq!(active.load(Ordering::Acquire), 0);
    assert!(try_admit(&active).is_some());
}

#[test]
fn short_deadline_rejects_open_partial_socket_without_waiting_for_idle_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let address = listener.local_addr().expect("address");
    let mut client = TcpStream::connect(address).expect("connect");
    let (server, _) = listener.accept().expect("accept");
    client.write_all(b"GET /health").expect("send partial line");
    let mut reader = BufReader::new(DeadlineRead {
        stream: server,
        until: Instant::now() + Duration::from_millis(150),
    });
    assert_eq!(parse_request(&mut reader), Err(IntakeError::TimedOut));
    drop(client);
}
