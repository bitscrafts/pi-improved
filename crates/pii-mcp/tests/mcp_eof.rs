//! R6 (spec 22): the MCP server has a BOUNDED lifetime — closing its stdin
//! (EOF) makes it exit cleanly (exit 0), instead of parking forever on
//! `pending()`. This is the integration test that pins the fix for the
//! "MCP client hang = server parks forever" caveat.
//!
//! Spawns the real `pii-mcp` server binary over stdio, immediately closes the
//! child's stdin, and asserts the process observes EOF and exits 0.
#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Write as _;
use std::process::{Command, Stdio};
use std::time::Duration;

/// Sends a minimal valid MCP `initialize` request over the child's stdin, then
/// signals EOF. This drives the server PAST the handshake (so `waiting()` is
/// servicing a live transport) before we close stdin — the exact shape of a
/// real Hermes shutdown. A server that parks forever on transport-close (the
/// R6 regression) hangs here and the test times out.
///
/// `Command::spawn` is allowed here because this integration test's entire
/// purpose IS to spawn the server binary over stdio and assert its lifetime.
#[test]
#[allow(clippy::disallowed_methods)] // reason: integration test spawns pii-mcp
fn mcp_server_exits_on_eof() {
    let bin = env!("CARGO_BIN_EXE_pii-mcp");
    let mut child = Command::new(bin)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn pii-mcp server");

    let mut stdin = child.stdin.take().expect("piped stdin");
    // Valid MCP initialize request (newline-delimited JSON-RPC 2.0).
    let init = br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"r6-eof-test","version":"1.0"}}}
"#;
    let _ = stdin.write_all(init);
    let _ = stdin.flush();
    // Give the current-thread runtime a moment to process the handshake and
    // reach a live transport state, THEN close stdin -> EOF.
    std::thread::sleep(Duration::from_millis(200));
    drop(stdin); // closes the pipe -> EOF for the server's stdio transport.

    // The server must exit on its own within a bounded window. EOF cleanup
    // should be near-instant; 15s is generous for a current-thread runtime.
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if std::time::Instant::now() > deadline => {
                let _ = child.kill();
                panic!(
                    "pii-mcp server did not exit on stdin EOF within 15s (R6 violated: parked forever)"
                );
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => panic!("try_wait failed: {e}"),
        }
    };
    assert!(
        status.success(),
        "pii-mcp server must exit 0 on stdin EOF (R6), got {status:?}"
    );
}
