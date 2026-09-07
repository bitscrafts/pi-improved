//! HTTP/1.1 codec integration tests (TDD table part-1): chunked vs
//! content-length equivalence, malformed/oversized refusals, https refusal.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use pii_abi::{HttpMethod, HttpRequest, HttpResponse};
use pii_net::http_codec::{parse_response, parse_url, render_request, render_response};

#[test]
fn http_parses_chunked_and_content_length() {
    let body = b"hello chunked world";
    let cl = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len());
    let mut cl_raw = cl.into_bytes();
    cl_raw.extend_from_slice(body);
    let chunked = format!(
        "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{}\r\n0\r\n\r\n",
        body.len(),
        String::from_utf8_lossy(body)
    );
    let a = parse_response(&cl_raw).unwrap();
    let b = parse_response(chunked.as_bytes()).unwrap();
    assert_eq!(a.status, 200);
    assert_eq!(a.body, b.body);
    assert_eq!(a.body, body.to_vec());
}

#[test]
fn http_rejects_oversized_and_malformed() {
    // 9 MiB body → refused, no panic.
    let big = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
        9 * 1024 * 1024
    );
    assert!(parse_response(big.as_bytes()).is_err());
    // HTTP/2 version → refused.
    assert!(parse_response(b"HTTP/2 200 OK\r\n\r\n").is_err());
    // Truncated status line → refused.
    assert!(parse_response(b"HTTP/1.").is_err());
    // Header block > 64 KiB → refused.
    let mut hdr = String::from("HTTP/1.1 200 OK\r\n");
    hdr.push_str("X-Pad: ");
    hdr.push_str(&"y".repeat(70_000));
    hdr.push_str("\r\n\r\n");
    assert!(parse_response(hdr.as_bytes()).is_err());
}

// Spec 17 (TLS 3b) replaced the Phase-3 refusal: `https://` now parses and
// dials through the rustls-wrapped stream. This test keeps the same slot in
// the suite (count may only go up — N3) but asserts the REPLACEMENT
// behavior: https parses with default port 443, and non-http(s) schemes are
// still refused with a scheme-naming error.
#[test]
fn https_is_an_explicit_refusal() {
    let (s, t, p) = parse_url("https://api.telegram.org/botX/getMe").unwrap();
    assert_eq!(s, pii_net::Scheme::Https, "https now parses (spec 17)");
    assert_eq!(t.host, "api.telegram.org");
    assert_eq!(t.port, 443);
    assert_eq!(p, "/botX/getMe");
    let err = parse_url("ftp://api.telegram.org/x").unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("scheme"), "names the scheme: {msg}");
}

#[test]
fn caller_headers_cannot_override_host_or_connection() {
    let req = HttpRequest {
        method: HttpMethod::Post,
        url: "http://fake.local:1234/api/chat".into(),
        headers: vec![
            (String::from("Host"), String::from("evil")),
            (String::from("Connection"), String::from("keep-alive")),
            (String::from("Authorization"), String::from("Bearer x")),
        ],
        body: b"{}".to_vec(),
        timeout_ms: 1_000,
    };
    let wire = String::from_utf8(render_request(&req, "fake.local:1234", "/api/chat")).unwrap();
    assert!(wire.starts_with("POST /api/chat HTTP/1.1\r\n"));
    assert!(wire.contains("Host: fake.local:1234"));
    assert!(!wire.contains("Host: evil"));
    assert!(wire.contains("Connection: close"));
    assert!(!wire.contains("keep-alive"));
    assert!(wire.contains("Authorization: Bearer x"));
    assert!(wire.contains("Content-Length: 2"));
}

#[test]
fn render_response_sets_content_length_once() {
    let r = HttpResponse {
        status: 200,
        headers: Vec::new(),
        body: b"ok".to_vec(),
    };
    let wire = String::from_utf8(render_response(&r)).unwrap();
    assert!(wire.contains("HTTP/1.1 200 OK"));
    assert_eq!(wire.matches("Content-Length").count(), 1);
}
