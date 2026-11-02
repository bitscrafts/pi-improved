//! Spec 331 R3: a `pii daemon` a test spawns is reaped even when the test panics. The control proves the
//! defect with the same daemon and no guard: a dropped `Child` leaves the process running, and the control
//! kills the pid it proved alive so it leaves nothing behind.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// reason: the test runs the built `pii` binary and `/bin/kill` to clean up the control's daemon.
#![allow(clippy::disallowed_methods)]

#[path = "daemon_guard/mod.rs"]
mod daemon_guard;

use daemon_guard::DaemonGuard;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Spawns the real `pii daemon` on a scratch workspace of its own, and returns the child with that directory.
fn spawn_daemon(prefix: &str) -> (Child, tempfile::TempDir) {
    let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("_tmp");
    std::fs::create_dir_all(&scratch).unwrap();
    let dir = tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(scratch)
        .unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_pii"))
        .arg("daemon")
        .args(["--sandbox", "off", "--workspace"])
        .arg(dir.path())
        .arg("--log-file")
        .arg(dir.path().join("daemon.log"))
        .env("PII_RUNTIME_DIR", dir.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    (child, dir)
}

fn proc_dir(pid: u32) -> PathBuf {
    PathBuf::from(format!("/proc/{pid}"))
}

/// True once the process has exited: its `/proc` entry is gone, or it is a zombie nobody reaps (no store, no socket).
fn is_gone(pid: u32) -> bool {
    std::fs::read_to_string(proc_dir(pid).join("stat")).map_or(true, |stat| {
        stat.rsplit(')')
            .next()
            .is_some_and(|rest| rest.trim_start().starts_with('Z'))
    })
}

/// Polls against a deadline, never a fixed sleep: returns as soon as the pid is gone, or false at the deadline.
fn gone_within(pid: u32, limit: Duration) -> bool {
    let end = Instant::now() + limit;
    while Instant::now() < end {
        if is_gone(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    is_gone(pid)
}

#[test]
fn a_panicking_test_leaves_no_daemon() {
    let (child, _dir) = spawn_daemon("guard-panic-");
    let pid = child.id();
    let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let _daemon = DaemonGuard::new(child);
        assert!(proc_dir(pid).exists(), "the daemon {pid} must be running");
        panic!("the test fails while the daemon is up");
    }));
    assert!(outcome.is_err(), "the closure must have panicked");
    assert!(
        gone_within(pid, Duration::from_secs(5)),
        "daemon {pid} survived the panic"
    );
}

#[test]
fn a_dropped_child_survives_without_the_guard() {
    let (child, _dir) = spawn_daemon("guard-control-");
    let pid = child.id();
    // The defect, measured: dropping a raw `Child` does not stop the process.
    #[allow(clippy::drop_non_drop)]
    // reason: the control drops a raw `Child` on purpose, to prove it leaks.
    drop(child);
    assert!(
        proc_dir(pid).exists(),
        "a dropped Child must leave daemon {pid} running (the defect)"
    );
    // The control cleans up the pid it proved alive, so it leaves no daemon.
    let status = Command::new("/bin/kill")
        .args(["-9", &pid.to_string()])
        .status()
        .unwrap();
    assert!(status.success(), "kill -9 {pid} failed: {status}");
    assert!(
        gone_within(pid, Duration::from_secs(5)),
        "daemon {pid} survived kill -9"
    );
}
