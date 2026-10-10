//! Spec 108 R3: a `pii tui` attach failure is reported as `tui.failure`
//! (production used the null observer, so it was dropped). The terminal is
//! never touched: the peer answers the hello with an error.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// reason: the test runs the built `pii` binary and binds a test socket.
#![allow(clippy::disallowed_methods)]

use std::path::Path;
use std::process::{Command, Stdio};

#[test]
fn an_attach_failure_is_reported_as_tui_failure_on_stderr() {
    let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("_tmp");
    std::fs::create_dir_all(&scratch).unwrap();
    let dir = tempfile::Builder::new()
        .prefix("tuif-")
        .tempdir_in(scratch)
        .unwrap();
    let sock = dir.path().join("c.sock");
    let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
    let t = std::thread::spawn(move || {
        use std::io::{BufRead, Write};
        let (stream, _) = listener.accept().unwrap();
        let mut line = String::new();
        std::io::BufReader::new(&stream)
            .read_line(&mut line)
            .unwrap();
        let reply = r#"{"jsonrpc":"2.0","id":0,"error":{"code":-32603,"message":"refused"}}"#;
        (&stream)
            .write_all(format!("{reply}\n").as_bytes())
            .unwrap();
        // hold the connection until the client has failed and exited
        let mut rest = String::new();
        let _ = std::io::BufReader::new(&stream).read_line(&mut rest);
    });
    let out = Command::new(env!("CARGO_BIN_EXE_pii"))
        .arg("tui")
        .arg("--connect")
        .arg(&sock)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    t.join().unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{err}");
    assert!(err.contains("tui.failure"), "stderr: {err}");
}
