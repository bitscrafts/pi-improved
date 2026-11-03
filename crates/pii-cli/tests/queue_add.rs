//! Spec 352 R1-R6: a row goes into the daemon's queue from outside (`pii queue add`), the daemon refuses the rows
//! that cannot be work, and the same queue is claimed through the daemon. Every step is one RPC; the CLI never
//! opens the store.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
// reason: the test runs the built `pii` binary and reads its stdout.
#![allow(clippy::disallowed_methods)]
// reason: the CONTROL_ test names are the spec contract.
#![allow(non_snake_case)]

#[path = "daemon_guard/mod.rs"]
mod daemon_guard;

use daemon_guard::DaemonGuard;
use std::path::Path;
use std::process::{Command, Output};
use std::time::{Duration, Instant};

fn pii(args: &[&str], rt: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pii"))
        .args(args)
        .env("PII_RUNTIME_DIR", rt)
        // State (and its durable stores) must be this test's own: the operator's are
        // held by any daemon already running on the machine.
        .env("PII_STATE_DIR", rt)
        .env("PII_CONNECT_TIMEOUT_MS", "500")
        .output()
        .unwrap()
}

fn until(what: &str, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(20);
    while Instant::now() < end {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("timed out waiting for {what}");
}

/// A daemon on `dir`, killed when the test ends, panic or not.
fn start_daemon(dir: &Path) -> DaemonGuard {
    let model = dir.join("model.json");
    std::fs::write(
        &model,
        r#"{"turns": [{"text": "ok", "tool_calls": [], "tokens_in": 1, "tokens_out": 1}]}"#,
    )
    .unwrap();
    let slot = daemon_guard::slot();
    let child = Command::new(env!("CARGO_BIN_EXE_pii"))
        .arg("daemon")
        .args(["--sandbox", "off", "--workspace"])
        .arg(dir)
        .arg("--log-file")
        .arg(dir.join("daemon.log"))
        .env("PII_RUNTIME_DIR", dir)
        .env("PII_STATE_DIR", dir)
        .env("PII_CONFIG", dir.join("pii.toml"))
        .env("PII_MODEL", format!("scripted:{}", model.display()))
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let daemon = DaemonGuard::new(child, slot);
    until("the daemon to answer", || {
        pii(&["sys", "status"], dir).status.success()
    });
    daemon
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn ls(dir: &Path) -> Vec<serde_json::Value> {
    let out = pii(&["queue", "ls"], dir);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    serde_json::from_slice(&out.stdout).unwrap()
}

/// `pii queue add --id <id> --spec <spec> --title t --criteria true`, with `spec` an existing file.
fn add(dir: &Path, id: &str, spec: &Path) -> Output {
    pii(
        &[
            "queue",
            "add",
            "--id",
            id,
            "--spec",
            spec.to_str().unwrap(),
            "--title",
            "t",
            "--criteria",
            "true",
        ],
        dir,
    )
}

fn existing_spec(dir: &Path) -> std::path::PathBuf {
    let spec = dir.join("spec.md");
    std::fs::write(&spec, "# spec\n").unwrap();
    spec
}

#[test]
fn a_row_added_from_the_cli_is_visible_to_the_daemon() {
    let d = tempfile::tempdir().unwrap();
    let dir = d.path();
    let _daemon = start_daemon(dir);
    let spec = existing_spec(dir);

    let out = add(dir, "W1", &spec);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let rows = ls(dir);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["id"], "W1");
    assert_eq!(rows[0]["state"], "pending");
}

#[test]
fn CONTROL_adding_the_same_id_twice_is_refused() {
    let d = tempfile::tempdir().unwrap();
    let dir = d.path();
    let _daemon = start_daemon(dir);
    let spec = existing_spec(dir);

    assert_eq!(add(dir, "W1", &spec).status.code(), Some(0));
    let again = add(dir, "W1", &spec);
    assert_eq!(again.status.code(), Some(2), "{}", stderr(&again));
    assert!(stderr(&again).contains("W1"), "{}", stderr(&again));
    let rows = ls(dir);
    assert_eq!(
        rows.len(),
        1,
        "a refused re-seed must not duplicate: {rows:?}"
    );
}

#[test]
fn CONTROL_a_row_whose_spec_does_not_exist_is_refused() {
    let d = tempfile::tempdir().unwrap();
    let dir = d.path();
    let _daemon = start_daemon(dir);

    let out = add(dir, "W1", Path::new("specs/does-not-exist.md"));
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("specs/does-not-exist.md"),
        "{}",
        stderr(&out)
    );
    assert_eq!(ls(dir), Vec::<serde_json::Value>::new());
}

#[test]
fn CONTROL_a_row_with_no_criteria_is_refused() {
    let d = tempfile::tempdir().unwrap();
    let dir = d.path();
    let _daemon = start_daemon(dir);
    let spec = existing_spec(dir);

    let out = pii(
        &[
            "queue",
            "add",
            "--id",
            "W1",
            "--spec",
            spec.to_str().unwrap(),
            "--title",
            "t",
        ],
        dir,
    );
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(stderr(&out).contains("nothing to run"), "{}", stderr(&out));
    assert_eq!(ls(dir), Vec::<serde_json::Value>::new());
}

#[test]
fn a_row_added_from_the_cli_is_claimed_by_the_select_node() {
    let d = tempfile::tempdir().unwrap();
    let dir = d.path();
    let _daemon = start_daemon(dir);
    let spec = existing_spec(dir);
    assert_eq!(add(dir, "W1", &spec).status.code(), Some(0));

    // The claim is the same `claim_next` the graph's `select` node calls, through the daemon.
    let out = pii(&["queue", "claim", "--by", "select"], dir);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let claimed: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(claimed["id"], "W1", "{claimed:?}");

    let log = std::fs::read_to_string(dir.join("daemon.log")).unwrap_or_default();
    assert!(log.contains("queue.item.added"), "{log}");
}
