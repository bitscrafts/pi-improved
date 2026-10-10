//! Spec 109 R1/R7: a daemon that accepts and stays silent does not hang the
//! CLI, and the terminal names the endpoint and the bound that expired.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// reason: the test runs the built `pii` binary and binds a test socket.
#![allow(clippy::disallowed_methods)]

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn scratch(prefix: &str) -> tempfile::TempDir {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("_tmp");
    std::fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(root)
        .unwrap()
}

fn pii_sys_status(runtime: &Path) -> (std::process::Output, Duration) {
    let t = Instant::now();
    let out = Command::new(env!("CARGO_BIN_EXE_pii"))
        .args(["sys", "status"])
        .env("PII_RUNTIME_DIR", runtime)
        .env("PII_CONNECT_TIMEOUT_MS", "400")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    (out, t.elapsed())
}

#[test]
fn a_silent_daemon_times_out_and_the_terminal_names_endpoint_and_bound() {
    let dir = scratch("silent-");
    let sock = dir.path().join("control.sock");
    let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let t = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let _ = rx.recv_timeout(Duration::from_secs(20)); // hold it open, say nothing
        drop(stream);
    });
    let (out, took) = pii_sys_status(dir.path());
    drop(tx);
    t.join().unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(took < Duration::from_secs(4), "took {took:?}");
    assert_ne!(out.status.code(), Some(0), "{err}");
    assert!(err.contains("proto.handshake.timeout"), "{err}");
    assert!(err.contains("\"bound_ms\":400"), "{err}");
    assert!(err.contains(&sock.display().to_string()), "{err}");
}

#[test]
fn a_missing_socket_is_reported_with_its_endpoint() {
    let dir = scratch("nosock-");
    let (out, _) = pii_sys_status(dir.path());
    let err = String::from_utf8_lossy(&out.stderr);
    assert_ne!(out.status.code(), Some(0), "{err}");
    assert!(err.contains("uds.connect.failed"), "{err}");
    assert!(err.contains("control.sock"), "{err}");
}
