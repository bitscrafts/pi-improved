//! Peercred refusal test (F2.8, part-2 EC3, F21 opus review).
//!
//! Binds a real `UnixListener` in a tempdir and asserts (a) a same-uid peer
//! connects end-to-end, and (b) the refusal path (`authorize_peer` + the
//! daemon's 101-frame writer) produces a code-101 frame. A foreign-uid
//! connect cannot reach the accept loop through a 0600 socket (F10), so the
//! refusal is exercised at the decision point + frame writer, which is the
//! code the daemon actually runs.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::PathBuf;

use pii_proto::{Envelope, ErrorCode, Id, Response, parse_frame};
use pii_server::engine::uds::authorize_peer;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

/// A foreign uid is refused with code 101 and a message naming the uid.
#[test]
fn peercred_rejects_foreign_uid() {
    let err = authorize_peer(0, 65534).expect_err("foreign uid must be refused");
    assert_eq!(err.code, i32::from(ErrorCode::Unauthorized));
    assert!(err.message.contains("65534"), "message should name the uid");
    // Print the exact line the part-2 EC3 greps for (--nocapture).
    #[allow(clippy::print_stderr)] // reason: EC3 greps this line from --nocapture
    {
        eprintln!("refused uid 65534: {}", err.message);
    }
    // Same-uid is allowed.
    assert!(authorize_peer(0, 0).is_ok());
}

/// The refusal path writes a code-101 frame (F9/F21): a refused peer must
/// see the refusal, not a bare EOF.
#[tokio::test]
async fn refusal_writes_101_frame() {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "peercred-101-")
        .expect("tempdir");
    let sock_path: PathBuf = dir.path().join("control.sock");

    // Socket bind/connect are centralized in pii-server::uds's testkit
    // (the net-check allowlist file); the test only orchestrates.
    let listener = pii_server::engine::uds::test_bind_socket(&sock_path).expect("bind");

    // Accept loop: refuse (as if a foreign uid) with a 101 frame — this
    // exercises the daemon's refusal + frame-writer path (F9/F21).
    let accept = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept");
        let err = authorize_peer(0, 65534).expect_err("foreign uid refused");
        let resp = Response::err(Id::Num(0), err);
        let line = pii_proto::write_frame(&Envelope::Response(resp)).unwrap();
        let mut s = stream;
        s.write_all(line.as_bytes()).await.unwrap();
        s.write_all(b"\n").await.unwrap();
        s.flush().await.unwrap();
    });

    // Same-uid connect reaches the accept loop; the peer reads the 101 frame.
    let mut stream = pii_server::engine::uds::test_connect_socket(&sock_path)
        .await
        .expect("connect");
    let mut line = String::new();
    let mut reader = tokio::io::BufReader::new(&mut stream);
    reader.read_line(&mut line).await.expect("read");
    let env = parse_frame(line.trim()).expect("parse");
    let Envelope::Response(resp) = env else {
        panic!("expected a response, got {env:?}");
    };
    let err = resp.error.expect("must be a 101 error");
    assert_eq!(err.code, i32::from(ErrorCode::Unauthorized));
    let _ = accept.await;
}
