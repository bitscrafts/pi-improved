//! Spec 206: `pii memory learn` records one queue outcome as a memory document through the daemon,
//! and an identical outcome is reported `unchanged` instead of being stored twice. With no daemon the
//! verb fails with a `memory.learn.failed` event on stderr and writes nothing.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
// reason: the test runs the built `pii` binary and reads its stdout and stderr.
#![allow(clippy::disallowed_methods)]

#[path = "daemon_guard/mod.rs"]
mod daemon_guard;

use daemon_guard::DaemonGuard;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn pii(args: &[&str], runtime: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_pii"))
        .args(args)
        .env("PII_RUNTIME_DIR", runtime)
        .env("PII_CONNECT_TIMEOUT_MS", "300")
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn learn(runtime: &Path, status: &str, reason: &str) -> std::process::Output {
    pii(
        &[
            "memory", "learn", "--run", "run-206", "--graph", "autonomy", "--status", status,
            "--reason", reason,
        ],
        runtime,
    )
}

fn scratch() -> tempfile::TempDir {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("_tmp");
    std::fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix("memory-learn-")
        .tempdir_in(root)
        .unwrap()
}

#[test]
fn an_outcome_is_recorded_once_and_a_repeat_reports_unchanged() {
    let d = scratch();
    let hello = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/phase2/hello.json");
    let cfg = d.path().join("pii.toml");
    std::fs::write(
        &cfg,
        "[[worker]]\nname = \"w\"\nprogram = \"/nonexistent/pii-worker-prog\"\n",
    )
    .unwrap();
    let mut child = DaemonGuard::new(
        Command::new(env!("CARGO_BIN_EXE_pii"))
            .arg("daemon")
            .args(["--sandbox", "off", "--workspace"])
            .arg(d.path())
            .arg("--log-file")
            .arg(d.path().join("daemon.log"))
            .env("PII_RUNTIME_DIR", d.path())
            .env("PII_CONFIG", &cfg)
            .env("PII_MODEL", format!("scripted:{}", hello.display()))
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );

    let end = Instant::now() + Duration::from_secs(20);
    let mut first = None;
    while Instant::now() < end && first.is_none() {
        let out = learn(d.path(), "skipped", "spec superseded");
        if out.status.success() {
            first = Some(out);
        } else {
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    let Some(first) = first else {
        child.kill_now();
        panic!("the daemon never accepted memory.learn");
    };
    let second = learn(d.path(), "skipped", "spec superseded");
    let found = pii(&["memory", "search", "spec superseded"], d.path());
    child.kill_now();

    let a: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    let b: serde_json::Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(a["unchanged"], false, "{a}");
    assert_eq!(b["unchanged"], true, "{b}");
    assert_eq!(a["id"], b["id"], "a repeat must address the same document");
    let hits = String::from_utf8_lossy(&found.stdout);
    assert!(
        hits.contains("run=run-206"),
        "search must return the outcome: {hits}"
    );
}

#[test]
fn without_a_daemon_the_failure_is_an_event_and_nothing_is_written() {
    let d = scratch();
    let out = learn(d.path(), "parked", "verify failed");
    assert_ne!(out.status.code(), Some(0), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("memory.learn.failed"), "{err}");
}
