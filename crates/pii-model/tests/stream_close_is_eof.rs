//! Spec 200: a provider stream that ends without `close_notify` AFTER at least one
//! frame is a completed body — the answer is kept and no `model.provider.error`
//! is emitted. A close before any frame stays an error.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    // reason: the test serves its own loopback peer, as in pii-net's tests.
    clippy::disallowed_methods
)]

use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpListener};
use std::sync::Arc;
use std::thread;

use pii_core::{Budget, CapKind, CapSet, Ctx};
use pii_model::{ChatMessage, ChatRequest, ModelProvider, ObservedProvider, OpenAiProvider, Role};
use pii_net::grant::{NetGrant, StaticGrants};
use pii_net::resolve::StaticResolver;
use pii_net::{NetConfig, NetService, SystemClock};
use pii_observe::{Recorder, names};

const FRAME: &str = "data: {\"choices\":[{\"delta\":{\"content\":\"PONG-GLM\"}}]}\n\n";

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

fn ctx() -> Ctx {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::NET;
    Ctx::root(caps, Budget::unlimited())
}

fn req() -> ChatRequest {
    ChatRequest {
        model: "m".into(),
        messages: vec![ChatMessage::new(Role::User, "hi")],
        tools: Vec::new(),
        max_tokens: 100,
        temperature: 0.5,
        num_ctx: 0,
    }
}

fn observed(net: NetService, base: &str, rec: &Arc<Recorder>) -> ObservedProvider {
    let inner = OpenAiProvider::new(Arc::new(net), base, "m".into(), None);
    ObservedProvider::new(Arc::new(inner), rec.clone())
}

#[test]
fn a_close_after_a_frame_completes_the_body_with_no_error() {
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
        tls.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\r\n")
            .unwrap();
        tls.write_all(FRAME.as_bytes()).unwrap();
        tls.flush().unwrap();
        drop(tls); // FIN without close_notify
    });
    let rec = Arc::new(Recorder::new());
    let p = observed(
        service("localhost", port),
        &format!("https://localhost:{port}/v1"),
        &rec,
    );
    let mut deltas = String::new();
    let resp = p
        .chat_streaming(&ctx(), &req(), &mut |d| deltas.push_str(d))
        .expect("a close after a frame is end of body, not an error");
    srv.join().unwrap();
    assert_eq!(resp.text, "PONG-GLM");
    assert_eq!(deltas, "PONG-GLM");
    assert_eq!(
        rec.events_named(names::MODEL_PROVIDER_ERROR),
        [] as [pii_observe::OwnedEvent; 0]
    );
}

#[test]
fn a_close_before_any_frame_is_still_an_error() {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let srv = thread::spawn(move || {
        let (s, _) = l.accept().unwrap();
        drop(s);
    });
    let rec = Arc::new(Recorder::new());
    let p = observed(
        service("silent.local", port),
        &format!("http://silent.local:{port}/v1"),
        &rec,
    );
    let r = p.chat_streaming(&ctx(), &req(), &mut |_| {});
    srv.join().unwrap();
    assert!(r.is_err(), "silence must not be an empty success");
    assert_eq!(rec.events_named(names::MODEL_PROVIDER_ERROR).len(), 1);
}
