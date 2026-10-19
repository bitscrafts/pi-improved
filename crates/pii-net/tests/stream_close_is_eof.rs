//! Spec 191: a peer close is not an error, driven through the production
//! `NetService::request_streaming` path.
//!   1. A TLS peer that sends a COMPLETE SSE answer and then drops the TCP
//!      socket WITHOUT `close_notify` is end of stream: the whole body is
//!      returned and no error surfaces.
//!   2. A close BEFORE any byte of the response is still an error.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    // reason: the test serves its own loopback peer; grants do not apply to
    // the test's own listener, as in budget_window.rs.
    clippy::disallowed_methods
)]

use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpListener};
use std::sync::Arc;
use std::thread;

use pii_abi::{HttpMethod, HttpRequest};
use pii_core::HttpTransport as _;
use pii_core::{Budget, CapKind, CapSet, Ctx};
use pii_net::grant::{NetGrant, StaticGrants};
use pii_net::resolve::StaticResolver;
use pii_net::{NetConfig, NetService, SystemClock};

const BODY: &str = "data: {\"answer\":\"PONG-GLM\"}\n\ndata: [DONE]\n\n";

fn service(host: &str, port: u16) -> NetService {
    let mut resolver = StaticResolver::new();
    resolver.insert(host, port, vec![SocketAddr::from(([127, 0, 0, 1], port))]);
    let mut grants = StaticGrants::default();
    grants.insert("*", "default", NetGrant::open_local());
    grants.insert("*", &format!("http:{host}"), NetGrant::open_local());
    NetService::new(NetConfig {
        resolver: Arc::new(resolver),
        grants: Arc::new(grants),
        clock: Arc::new(SystemClock),
    })
}

fn request(url: String) -> HttpRequest {
    HttpRequest {
        method: HttpMethod::Post,
        url,
        headers: Vec::new(),
        body: b"{}".to_vec(),
        timeout_ms: 5_000,
    }
}

fn ctx() -> Ctx {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::NET;
    Ctx::root(caps, Budget::unlimited())
}

#[test]
fn an_abrupt_tls_close_after_a_full_answer_is_end_of_stream_not_error() {
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    pii_net::dial::trust_test_root(cert.cert.der().clone());
    let key = rustls::pki_types::PrivateKeyDer::Pkcs8(cert.signing_key.serialize_der().into());
    let cfg = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert.cert.der().clone()], key)
        .unwrap();
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let srv = thread::spawn(move || {
        let (s, _) = l.accept().unwrap();
        let conn = rustls::ServerConnection::new(Arc::new(cfg)).unwrap();
        let mut tls = rustls::StreamOwned::new(conn, s);
        let mut seen = Vec::new();
        let mut buf = [0u8; 4096];
        while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = tls.read(&mut buf).unwrap();
            assert!(n > 0, "client closed before sending its request");
            seen.extend_from_slice(&buf[..n]);
        }
        let head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\r\n";
        tls.write_all(head.as_bytes()).unwrap();
        tls.write_all(BODY.as_bytes()).unwrap();
        tls.flush().unwrap();
        drop(tls); // INJECTION: FIN without close_notify
    });

    let net = service("localhost", port);
    let mut streamed = Vec::new();
    let resp = net
        .request_streaming(
            &ctx(),
            &request(format!("https://localhost:{port}/v1/chat/completions")),
            &mut |c| streamed.extend_from_slice(c),
        )
        .expect("an abrupt close after a full answer must not be an error");
    srv.join().unwrap();
    assert_eq!(resp.status, 200);
    assert_eq!(String::from_utf8_lossy(&resp.body), BODY);
    assert_eq!(String::from_utf8_lossy(&streamed), BODY);
}

#[test]
fn a_close_before_any_byte_is_still_an_error_truncated_response() {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let srv = thread::spawn(move || {
        let (s, _) = l.accept().unwrap();
        drop(s); // accept-and-drop, nothing ever sent
    });
    let net = service("silent.local", port);
    let resp = net.request_streaming(
        &ctx(),
        &request(format!("http://silent.local:{port}/v1/chat/completions")),
        &mut |_| {},
    );
    srv.join().unwrap();
    let e = resp.expect_err("silence must not be an empty success");
    let m = e.to_string();
    assert!(
        m.contains("truncated response") || m.contains("net: read"),
        "the error must name what happened: {m}"
    );
}
