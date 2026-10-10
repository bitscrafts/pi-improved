//! Spec 109 R1/R7: the handshake never hangs, and every failure is reported.
#![cfg(feature = "client")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::many_single_char_names
)]

use std::time::{Duration, Instant};

use pii_observe::{Recorder, names};
use pii_proto::client::{Client, ClientError, ConnectOptions};
use pii_proto::sys::PeerInfo;

const BOUND: Duration = Duration::from_millis(300);
const SLACK: Duration = Duration::from_millis(700);

fn info() -> PeerInfo {
    PeerInfo {
        name: "t".into(),
        version: "0".into(),
        build: "t".into(),
    }
}

fn opts(rec: &Recorder) -> ConnectOptions<'_> {
    ConnectOptions {
        endpoint: "/run/test/control.sock",
        bound: BOUND,
        observer: rec,
    }
}

#[tokio::test]
async fn peer_that_closes_without_answering_is_an_error() {
    let rec = Recorder::new();
    let (c, s) = tokio::io::duplex(1024);
    drop(s);
    let (r, w) = tokio::io::split(c);
    let t = Instant::now();
    let err = Client::connect_with(r, w, info(), &opts(&rec))
        .await
        .err()
        .expect("must fail");
    let took = t.elapsed();
    assert!(took < BOUND + SLACK, "took {took:?}");
    assert!(
        matches!(&err, ClientError::Closed { endpoint } if endpoint == "/run/test/control.sock"),
        "{err}"
    );
    let ev = rec.events_named(names::PROTO_HANDSHAKE_CLOSED);
    assert_eq!(ev.len(), 1);
    assert_eq!(
        ev[0]
            .fields
            .get("endpoint")
            .and_then(serde_json::Value::as_str),
        Some("/run/test/control.sock")
    );
}

#[tokio::test]
async fn silent_peer_errors_at_the_bound() {
    let rec = Recorder::new();
    let (c, _s) = tokio::io::duplex(1024);
    let (r, w) = tokio::io::split(c);
    let t = Instant::now();
    let err = Client::connect_with(r, w, info(), &opts(&rec))
        .await
        .err()
        .expect("must fail");
    let took = t.elapsed();
    assert!(took >= BOUND, "returned before the bound: {took:?}");
    assert!(took < BOUND + SLACK, "took {took:?}");
    assert!(
        matches!(&err, ClientError::Timeout { endpoint, bound_ms }
            if endpoint == "/run/test/control.sock" && *bound_ms == 300),
        "{err}"
    );
    let ev = rec.events_named(names::PROTO_HANDSHAKE_TIMEOUT);
    assert_eq!(ev.len(), 1);
    assert_eq!(
        ev[0]
            .fields
            .get("bound_ms")
            .and_then(serde_json::Value::as_u64),
        Some(300)
    );
}

#[tokio::test]
async fn protocol_skew_is_reported_with_both_versions() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let rec = Recorder::new();
    let (c, s) = tokio::io::duplex(8192);
    let (r, w) = tokio::io::split(c);
    let server = tokio::spawn(async move {
        let (sr, mut sw) = tokio::io::split(s);
        let mut line = String::new();
        BufReader::new(sr).read_line(&mut line).await.unwrap();
        let req: serde_json::Value = serde_json::from_str(&line).unwrap();
        let resp = serde_json::json!({"jsonrpc":"2.0","id":req["id"],"error":{"code":100,"message":"skew","data":{"got":"9.9"}}});
        sw.write_all(format!("{resp}\n").as_bytes()).await.unwrap();
        sw.flush().await.unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
    });
    let err = Client::connect_with(r, w, info(), &opts(&rec))
        .await
        .err()
        .expect("must fail");
    assert!(matches!(err, ClientError::ProtocolMismatch { .. }), "{err}");
    let ev = rec.events_named(names::PROTO_HANDSHAKE_SKEW);
    assert_eq!(ev.len(), 1);
    assert_eq!(
        ev[0].fields.get("got").and_then(serde_json::Value::as_str),
        Some("9.9")
    );
    assert!(ev[0].fields.get("want").is_some());
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn closed_peer_never_hangs_across_many_attempts() {
    for _ in 0..2000 {
        let (c, s) = tokio::io::duplex(1024);
        drop(s);
        let (r, w) = tokio::io::split(c);
        assert!(Client::connect(r, w, info()).await.is_err());
    }
}

#[test]
fn the_default_bound_is_short_enough_to_be_useful() {
    assert_eq!(
        pii_proto::client::DEFAULT_CONNECT_TIMEOUT,
        Duration::from_secs(5)
    );
    assert_eq!(
        ConnectOptions::new("x").bound,
        pii_proto::client::DEFAULT_CONNECT_TIMEOUT
    );
}
