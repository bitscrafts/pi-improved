//! Spec 217 R6: `pii run trace <run_id>` prints one line per node, built from journal events only, with the agent
//! call's pid, image digest, criteria fraction and failing criterion name, and a summary with the orphan count.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
// reason: the test runs the built `pii` binary and reads its stdout.
#![allow(clippy::disallowed_methods)]

use pii_abi::{NodeId, RunEvent, RunHeader, RunId, RunStatus, Terminal};
use pii_store::Store;

const PROCESS_START_MS: u64 = 1_000;

fn observed(kind: &str, fields: serde_json::Value) -> RunEvent {
    RunEvent::NodeObserved {
        node: NodeId::from("code"),
        kind: kind.to_owned(),
        fields,
    }
}

/// Records one run with a single coder call: spawned, exited with a failing second criterion, and finished.
fn record_run(path: &std::path::Path) {
    let store = Store::open(path).unwrap();
    let mut header = RunHeader {
        run: RunId::from("run-trace"),
        graph: "trace-test".to_owned(),
        status: RunStatus::Running,
        seq: 0,
        dur_ms: 0,
        revision: String::new(),
        process_start_ms: PROCESS_START_MS,
        created_ms: PROCESS_START_MS,
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
                "criteria_passed": 1,
                "criteria_failed": 1,
                "first_failure": "tests-green",
            }),
        ),
        RunEvent::NodeFinished {
            node: NodeId::from("code"),
            port: pii_abi::PortName::from("err"),
            value: serde_json::json!({ "reason": "criteria failed" }),
            value_hash: String::new(),
            at_ms: 10,
            dur_ms: 1200,
        },
    ];
    for event in &events {
        store.append_event(&mut header, event).unwrap();
    }
    header.status = RunStatus::Failed;
    store
        .append_event(
            &mut header,
            &RunEvent::Finished {
                terminal: Terminal::Failure {
                    error: "criteria failed".to_owned(),
                },
                output_hash: String::new(),
            },
        )
        .unwrap();
}

#[test]
fn run_trace_names_the_call() {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "trace-cli-").unwrap();
    let db = dir.path().join("store.redb");
    record_run(&db);

    // Its own runtime dir: a daemon on this host must not answer for the store this test names (spec 220).
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_pii"))
        .args(["run", "trace", "run-trace", "--db"])
        .arg(&db)
        .env("PII_RUNTIME_DIR", dir.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "trace exits zero: {:?} {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let node_line = stdout
        .lines()
        .find(|l| l.contains(" code "))
        .unwrap_or_else(|| panic!("a line for node code: {stdout}"));
    for want in [
        "pid=4242",
        "image=abcdef01",
        "crit=1/2",
        "fail=tests-green",
        "rc=0",
        "10+1200",
    ] {
        assert!(node_line.contains(want), "{want} in `{node_line}`");
    }
    assert!(
        stdout
            .lines()
            .any(|l| l == "trace: 1 nodes · 1 agent calls · 0 orphans"),
        "summary line with orphan count: {stdout}"
    );
}
