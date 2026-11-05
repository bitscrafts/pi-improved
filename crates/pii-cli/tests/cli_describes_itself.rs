//! Spec 382: the CLI's own help, usage and refusals name what the binary really does.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// reason: the test runs the built `pii` binary with a scrubbed environment.
#![allow(clippy::disallowed_methods)]

use std::process::{Command, Stdio};

mod daemon_guard;
use std::time::{Duration, Instant};

/// A `pii` command with an isolated runtime dir, so no real daemon is reached.
fn pii(dir: &std::path::Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_pii"));
    cmd.current_dir(dir)
        .env("PII_RUNTIME_DIR", dir.join("run"))
        .env_remove("PII_CONFIG")
        .stdin(Stdio::null());
    cmd
}

/// R1: `run ls --status` accepts every status the store holds, `refused` included.
#[test]
fn run_ls_accepts_refused_and_names_the_full_set() {
    let dir = tempfile::tempdir().unwrap();
    let out = pii(dir.path())
        .args(["run", "ls", "--status", "refused"])
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        !err.contains("unknown --status"),
        "refused is not accepted: {err}"
    );

    let out = pii(dir.path())
        .args(["run", "ls", "--status", "nope"])
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        err.contains("running|suspended|succeeded|failed|refused"),
        "accepted set not named: {err}"
    );
}

/// R2: a `--config` path that does not exist is refused, naming the path, and nothing is created.
#[test]
fn daemon_refuses_a_missing_config_and_names_it() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing.toml");
    // Spec 331: a test never kills a daemon by hand. The guard owns the child and kills it on Drop,
    // panic or not; the deadline below only decides what the assertion says.
    let mut cmd = pii(dir.path());
    cmd.arg("daemon")
        .arg("--config")
        .arg(&missing)
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut guard = daemon_guard::spawn(&mut cmd);
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        if let Some(status) = guard.child_mut().try_wait().unwrap() {
            break Some(status);
        }
        if Instant::now() > deadline {
            break None; // the guard kills it on drop
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let status = status.expect("daemon ran on a missing --config instead of refusing it");
    let mut err = String::new();
    std::io::Read::read_to_string(&mut guard.child_mut().stderr.take().unwrap(), &mut err).unwrap();
    assert_eq!(status.code(), Some(2), "stderr: {err}");
    assert!(
        err.contains(&missing.display().to_string()),
        "path not named: {err}"
    );
    assert!(!missing.exists(), "--config created the file");
}

/// R3: the store verbs that need the daemon stopped say so, and `stats` is among them.
#[test]
fn store_usage_marks_stats_as_needing_the_daemon_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let out = pii(dir.path()).arg("store").output().unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    let marked = err
        .lines()
        .find(|l| l.contains("daemon must be stopped"))
        .unwrap_or_default();
    // Only the parenthesised note counts: the verb list above it always names `stats`.
    let note = marked.rsplit('(').next().unwrap_or_default();
    assert!(note.contains("stats"), "stats not marked: {err}");
}

/// R4: the `pii graph` usage lists every subcommand it dispatches, `check` included.
#[test]
fn graph_usage_lists_check() {
    let dir = tempfile::tempdir().unwrap();
    let out = pii(dir.path()).arg("graph").output().unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("check"), "check missing from usage: {err}");
}
