//! Spec 108 R3: `pii daemon` attaches its log to the subsystems it builds. A
//! worker state change (a subsystem built AFTER the engine) must appear in
//! the daemon's log file — proof that `attach_log` runs in the real start-up
//! path, not only in a hand-built test engine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// reason: the test runs the built `pii` binary.
#![allow(clippy::disallowed_methods)]

#[path = "daemon_guard/mod.rs"]
mod daemon_guard;

use daemon_guard::DaemonGuard;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

#[test]
fn a_worker_state_change_reaches_the_daemon_log_file() {
    let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("_tmp");
    std::fs::create_dir_all(&scratch).unwrap();
    let dir = tempfile::Builder::new()
        .prefix("dlog-")
        .tempdir_in(scratch)
        .unwrap();
    let hello = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/phase2/hello.json");
    let cfg = dir.path().join("pii.toml");
    std::fs::write(
        &cfg,
        "[[worker]]\nname = \"w\"\nprogram = \"/nonexistent/pii-worker-prog\"\n",
    )
    .unwrap();
    let log = dir.path().join("daemon.log");
    let mut child = DaemonGuard::new(
        Command::new(env!("CARGO_BIN_EXE_pii"))
            .arg("daemon")
            .args(["--sandbox", "off", "--workspace"])
            .arg(dir.path())
            .arg("--log-file")
            .arg(&log)
            .env("PII_RUNTIME_DIR", dir.path())
            .env("PII_CONFIG", &cfg)
            .env("PII_MODEL", format!("scripted:{}", hello.display()))
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    let end = Instant::now() + Duration::from_secs(15);
    let mut found = false;
    while Instant::now() < end {
        if std::fs::read_to_string(&log).is_ok_and(|t| t.contains("worker.state.changed")) {
            found = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    child.kill_now();
    assert!(
        found,
        "log: {}",
        std::fs::read_to_string(&log).unwrap_or_default()
    );
}
