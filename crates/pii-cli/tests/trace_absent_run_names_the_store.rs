//! Spec 220 R2/R4/R5: a run id that is not there is a named failure, and the message says which store was consulted,
//! so "no run" is distinguishable from "wrong store".

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
// reason: the test runs the built `pii` binary and reads its stderr.
#![allow(clippy::disallowed_methods)]

#[path = "daemon_guard/mod.rs"]
mod daemon_guard;

use daemon_guard::DaemonGuard;
use std::path::Path;
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use pii_abi::{NodeId, RunEvent, RunHeader, RunId, RunStatus};
use pii_store::Store;
use pii_store::testing::tmp_scratch;

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
        // State (and its durable stores) must be this test's own: the operator's are
        // held by any daemon already running on the machine.
        .env("PII_STATE_DIR", dir)
        // The daemon must not read the operator's ~/.pii/pii.toml: its [[cron]] entries arm at
        // boot and park suspended runs in this scratch store (measured 2026-10-20: the failing
        // "has 1 runs" count was 2-3, wall-clock dependent). A missing file is an empty config.
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

/// Creates a store at `path` holding one run, `run-present`, and closes it.
fn seed_one_run(path: &Path) {
    let store = Store::open(path).unwrap();
    let mut header = RunHeader {
        run: RunId::from("run-present"),
        graph: "present".to_owned(),
        status: RunStatus::Running,
        seq: 0,
        dur_ms: 0,
        revision: String::new(),
        process_start_ms: 1_000,
        // Inside the retention policy, so the daemon's boot sweep (spec 376 R4) keeps it and the
        // count the message reports is the one this test seeded. An epoch value would be swept as
        // ancient, which is correct behaviour and made this fixture assert the wrong thing.
        created_ms: u64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        )
        .unwrap(),
    };
    store
        .append_event(
            &mut header,
            &RunEvent::NodeStarted {
                node: NodeId::from("n"),
                at_ms: 10,
            },
        )
        .unwrap();
}

#[test]
fn absent_run_names_the_store_it_opened_when_no_daemon_answers() {
    let tmp = tmp_scratch(env!("CARGO_MANIFEST_DIR"), "trace-absent-offline-").unwrap();
    let dir = tmp.path();
    let db = dir.join("elsewhere.redb");
    seed_one_run(&db);
    let db_arg = db.to_str().unwrap();

    let out = pii(&["run", "trace", "r-missing", "--db", db_arg], dir);
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        !out.status.success(),
        "an absent run is a failure: {stderr}"
    );
    assert!(stderr.contains("no run r-missing"), "{stderr}");
    assert!(
        stderr.contains(db_arg),
        "the message names the store it opened: {stderr}"
    );
}

#[test]
fn absent_run_names_the_daemon_store_and_its_run_count() {
    let tmp = tmp_scratch(env!("CARGO_MANIFEST_DIR"), "trace-absent-daemon-").unwrap();
    let dir = tmp.path();
    seed_one_run(&dir.join("store.redb"));
    let _daemon = start_daemon(dir);

    let out = pii(&["run", "trace", "r-missing"], dir);
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        !out.status.success(),
        "an absent run is a failure: {stderr}"
    );
    assert!(
        stderr.contains("store.redb"),
        "the message names the daemon's store: {stderr}"
    );
    assert!(
        stderr.contains("has 1 runs"),
        "the message gives the daemon's run count: {stderr}"
    );
}
