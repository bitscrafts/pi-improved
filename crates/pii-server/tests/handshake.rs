//! Handshake tests (F1.9, part-3 EC3): version skew is a hard refusal on
//! both sides, and any method before a successful hello is refused.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::PathBuf;
use std::sync::Arc;

use pii_proto::method::Method;
use pii_proto::session::SessionCreateParams;
use pii_proto::sys::PeerInfo;
use pii_proto::{Envelope, ErrorCode, Id, PROTOCOL_VERSION, Request, parse_frame};
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

fn engine_in(dir: &std::path::Path) -> Arc<Engine> {
    Engine::new(EngineConfig {
        workspace: dir.to_path_buf(),
        store_path: dir.join("store.redb"),
        model: format!("scripted:{}", fixtures_hello().display()),
        sandbox: SandboxProfileName::Off,
        graph_dir: dir.join("graphs"),
    })
    .expect("engine")
}

fn fixtures_hello() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .join("fixtures/phase2/hello.json")
}

/// A hello with `protocol:"pii.v2"` is refused with code 100, data.expected
/// == "pii.v1", and the connection is closed (no session created).
#[tokio::test]
async fn hello_skew_is_hard_refusal() {
    let dir = tempfile::Builder::new()
        .prefix("handshake-")
        .tempdir_in("/projects/pi-improved/_tmp")
        .expect("tempdir");
    let engine = engine_in(dir.path());
    let stream = pii_server::loopback::spawn_loopback(engine);
    let (mut r, mut w) = tokio::io::split(stream);

    // Send a hello with the WRONG protocol version.
    let req = Request {
        jsonrpc: "2.0".into(),
        id: Id::Num(1),
        method: Method::SysHello,
        params: serde_json::json!({
            "protocol": "pii.v2",
            "client": {"name": "test", "version": "0", "build": "test"},
            "encodings": ["json"],
            "features": []
        }),
    };
    let line = serde_json::to_string(&Envelope::Request(req)).expect("ser");
    w.write_all(line.as_bytes()).await.expect("write");
    w.write_all(b"\n").await.expect("write nl");
    w.flush().await.expect("flush");

    // Read the response.
    let mut reader = tokio::io::BufReader::new(&mut r);
    let mut resp_line = String::new();
    reader.read_line(&mut resp_line).await.expect("read");
    let env = parse_frame(resp_line.trim()).expect("parse");
    let Envelope::Response(resp) = env else {
        panic!("expected a response, got {env:?}");
    };
    let err = resp.error.expect("skew must be an error");
    assert_eq!(err.code, i32::from(ErrorCode::ProtocolMismatch));
    assert_eq!(
        err.data
            .as_ref()
            .and_then(|d| d.get("expected"))
            .and_then(|v| v.as_str()),
        Some(PROTOCOL_VERSION)
    );

    // The connection is closed: no further frames arrive.
    let mut more = String::new();
    let n = reader.read_line(&mut more).await.expect("read eof");
    assert_eq!(n, 0, "connection must be closed after skew refusal");
}

/// A method other than `sys.hello` before a successful hello is refused
/// with code 101 and the connection is closed.
#[tokio::test]
async fn method_before_hello_refused() {
    let dir = tempfile::Builder::new()
        .prefix("handshake-")
        .tempdir_in("/projects/pi-improved/_tmp")
        .expect("tempdir");
    let engine = engine_in(dir.path());
    let stream = pii_server::loopback::spawn_loopback(engine);
    let (mut r, mut w) = tokio::io::split(stream);

    let req = Request {
        jsonrpc: "2.0".into(),
        id: Id::Num(1),
        method: Method::SessionCreate,
        params: serde_json::to_value(SessionCreateParams {
            cwd: "/".into(),
            model: None,
            tools: Vec::new(),
            graph: None,
        })
        .expect("params"),
    };
    let line = serde_json::to_string(&Envelope::Request(req)).expect("ser");
    w.write_all(line.as_bytes()).await.expect("write");
    w.write_all(b"\n").await.expect("write nl");
    w.flush().await.expect("flush");

    let mut reader = tokio::io::BufReader::new(&mut r);
    let mut resp_line = String::new();
    reader.read_line(&mut resp_line).await.expect("read");
    let env = parse_frame(resp_line.trim()).expect("parse");
    let Envelope::Response(resp) = env else {
        panic!("expected a response, got {env:?}");
    };
    let err = resp.error.expect("must be an error");
    assert_eq!(err.code, i32::from(ErrorCode::Unauthorized));

    let mut more = String::new();
    let n = reader.read_line(&mut more).await.expect("read eof");
    assert_eq!(n, 0, "connection must be closed after pre-hello method");
}

/// A valid hello succeeds and a session can be created.
#[tokio::test]
async fn valid_hello_creates_session() {
    let dir = tempfile::Builder::new()
        .prefix("handshake-")
        .tempdir_in("/projects/pi-improved/_tmp")
        .expect("tempdir");
    let engine = engine_in(dir.path());
    let stream = pii_server::loopback::spawn_loopback(engine);
    let (r, w) = tokio::io::split(stream);
    let client_info = PeerInfo {
        name: "test".into(),
        version: "0".into(),
        build: "test".into(),
    };
    let (client, hello) = pii_proto::client::Client::connect(r, w, client_info)
        .await
        .expect("connect");
    assert_eq!(hello.protocol, PROTOCOL_VERSION);
    let sid = client
        .call(
            Method::SessionCreate,
            serde_json::to_value(SessionCreateParams {
                cwd: "/".into(),
                model: None,
                tools: Vec::new(),
                graph: None,
            })
            .expect("params"),
        )
        .await
        .expect("session.create");
    assert!(sid.get("session_id").is_some());
}

/// A hello offering only non-`json` encodings is refused (encoding
/// negotiation is cut to a refusal; the server speaks json only).
#[tokio::test]
async fn hello_non_json_encoding_refused() {
    let dir = tempfile::Builder::new()
        .prefix("handshake-")
        .tempdir_in("/projects/pi-improved/_tmp")
        .expect("tempdir");
    let engine = engine_in(dir.path());
    let stream = pii_server::loopback::spawn_loopback(engine);
    let (mut r, mut w) = tokio::io::split(stream);

    let req = Request {
        jsonrpc: "2.0".into(),
        id: Id::Num(1),
        method: Method::SysHello,
        params: serde_json::json!({
            "protocol": PROTOCOL_VERSION,
            "client": {"name": "test", "version": "0", "build": "test"},
            "encodings": ["postcard"],
            "features": []
        }),
    };
    let line = serde_json::to_string(&Envelope::Request(req)).expect("ser");
    w.write_all(line.as_bytes()).await.expect("write");
    w.write_all(b"\n").await.expect("write nl");
    w.flush().await.expect("flush");

    let mut reader = tokio::io::BufReader::new(&mut r);
    let mut resp_line = String::new();
    reader.read_line(&mut resp_line).await.expect("read");
    let env = parse_frame(resp_line.trim()).expect("parse");
    let Envelope::Response(resp) = env else {
        panic!("expected a response, got {env:?}");
    };
    let err = resp.error.expect("non-json hello must be refused");
    assert_eq!(err.code, i32::from(ErrorCode::InvalidParams));
    assert!(err.message.contains("encoding"), "{}", err.message);
}
