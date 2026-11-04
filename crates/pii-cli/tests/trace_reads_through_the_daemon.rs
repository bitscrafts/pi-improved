//! Spec 220 R1/R4: `pii run trace` and `pii run explain` read the run through the running daemon, so a run is
//! readable while the daemon owns the store, and the output says which source answered.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
// reason: the test runs the built `pii` binary and reads its stdout.
#![allow(clippy::disallowed_methods)]

#[path = "daemon_guard/mod.rs"]
mod daemon_guard;

use daemon_guard::DaemonGuard;
use std::path::Path;
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use pii_abi::{NodeId, PortName, RunEvent, RunHeader, RunId, RunStatus, Terminal};
use pii_store::Store;
use pii_store::testing::tmp_scratch;

const RUN: &str = "run-daemon";

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

/// Writes one finished run with a single coder call (spawned, then exited with every criterion passing) into the
/// store at `path`. The store is closed when this returns, so a daemon can open it next.
fn seed(path: &Path) {
    let store = Store::open(path).unwrap();
    let mut header = RunHeader {
        run: RunId::from(RUN),
        graph: "trace-daemon".to_owned(),
        status: RunStatus::Running,
        seq: 0,
        dur_ms: 0,
        revision: String::new(),
        process_start_ms: 1_000,
        // Dated now: a boot sweep (spec 376) drops runs that look ancient.
        created_ms: u64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        )
        .unwrap(),
    };
    let observed = |kind: &str, fields: serde_json::Value| RunEvent::NodeObserved {
        node: NodeId::from("code"),
        kind: kind.to_owned(),
        fields,
    };
    let events = [
        RunEvent::NodeStarted {
            node: NodeId::from("code"),
            at_ms: 10,
        },
        observed(
            "coder.spawned",
            serde_json::json!({
                "pid": 4242,
                "agent": "pi",
                "model": "claude-test",
                "image": "pii/agent:1",
                "image_digest": "sha256:abcdef0123456789abcdef0123456789",
                "argv_digest": "00",
            }),
        ),
        observed(
            "coder.exited",
            serde_json::json!({
                "pid": 4242,
                "rc": 0,
                "dur_ms": 1200,
                "stop_reason": "end_turn",
                "criteria_run": 2,
                "criteria_passed": 2,
                "criteria_failed": 0,
                "first_failure": "",
            }),
        ),
        RunEvent::NodeFinished {
            node: NodeId::from("code"),
            port: PortName::from("ok"),
            value: serde_json::json!({}),
            value_hash: String::new(),
            at_ms: 10,
            dur_ms: 1200,
        },
    ];
    for event in &events {
        store.append_event(&mut header, event).unwrap();
    }
    header.status = RunStatus::Succeeded;
    store
        .append_event(
            &mut header,
            &RunEvent::Finished {
                terminal: Terminal::Success {
                    output: serde_json::json!({}),
                },
                output_hash: String::new(),
            },
        )
        .unwrap();
}

#[test]
fn trace_and_explain_read_the_run_through_the_daemon() {
    let tmp = tmp_scratch(env!("CARGO_MANIFEST_DIR"), "trace-daemon-").unwrap();
    let dir = tmp.path();
    seed(&dir.join("store.redb"));
    let _daemon = start_daemon(dir);

    // The daemon holds the store, so a direct open would fail on the lock. The events still come back.
    let out = pii(&["run", "trace", RUN], dir);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(out.status.success(), "trace exits zero: {stderr}");
    assert!(!stderr.contains("lock"), "no lock failure: {stderr}");
    assert!(
        stdout
            .lines()
            .any(|l| l.contains(" code coder ") && l.contains("pid=4242")),
        "the coder call is printed from the daemon: {stdout}"
    );
    assert!(
        stdout.lines().any(|l| l == "via=daemon"),
        "the source is printed: {stdout}"
    );

    let out = pii(&["run", "explain", RUN], dir);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(out.status.success(), "explain exits zero: {stdout}");
    assert!(
        stdout.contains("code ran pi on model claude-test"),
        "the explain sentence names the call: {stdout}"
    );
    assert!(
        stdout.lines().any(|l| l == "via=daemon"),
        "explain prints its source too: {stdout}"
    );
}
