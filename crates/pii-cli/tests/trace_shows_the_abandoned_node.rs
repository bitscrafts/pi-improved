//! Spec 310 R2: `pii run trace` prints a line for a node that started and never finished (its run was cancelled while the
//! node's agent ran), with the agent call it made; a node that finished keeps exactly one line.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    non_snake_case
)]
// reason: the test runs the built `pii` binary and reads its stdout.
#![allow(clippy::disallowed_methods)]

use pii_abi::{NodeId, RunEvent, RunHeader, RunId, RunStatus, Terminal};
use pii_store::Store;

const PROCESS_START_MS: u64 = 1_000;

fn spawned(node: &str, pid: i64) -> RunEvent {
    RunEvent::NodeObserved {
        node: NodeId::from(node),
        kind: "coder.spawned".to_owned(),
        fields: serde_json::json!({
            "pid": pid,
            "agent": "pi",
            "image": "pii/agent:1",
            "image_digest": "sha256:abcdef0123456789abcdef0123456789",
            "argv_digest": "00",
        }),
    }
}

/// Records a run whose node `leaf` started and spawned an agent (pid 971276). The run is then closed as failed; the
/// node finished only when `finish_leaf` is set.
fn record_run(path: &std::path::Path, run: &str, finish_leaf: bool) {
    let store = Store::open(path).unwrap();
    let mut header = RunHeader {
        run: RunId::from(run),
        graph: "chain-test".to_owned(),
        status: RunStatus::Running,
        seq: 0,
        dur_ms: 0,
        revision: String::new(),
        process_start_ms: PROCESS_START_MS,
        created_ms: PROCESS_START_MS,
    };
    let mut events = vec![
        RunEvent::NodeStarted {
            node: NodeId::from("leaf"),
            at_ms: 10,
        },
        spawned("leaf", 971_276),
    ];
    if finish_leaf {
        events.push(RunEvent::NodeFinished {
            node: NodeId::from("leaf"),
            port: pii_abi::PortName::from("err"),
            value: serde_json::json!({ "reason": "criteria failed" }),
            value_hash: String::new(),
            at_ms: 10,
            dur_ms: 1200,
        });
    }
    for event in &events {
        store.append_event(&mut header, event).unwrap();
    }
    header.status = RunStatus::Failed;
    store
        .append_event(
            &mut header,
            &RunEvent::Finished {
                terminal: Terminal::Failure {
                    error: "cancelled: test".to_owned(),
                },
                output_hash: String::new(),
            },
        )
        .unwrap();
}

/// Runs `pii run trace <run>` on the store at `path` and returns its stdout.
fn trace(dir: &std::path::Path, path: &std::path::Path, run: &str) -> String {
    // Its own runtime dir: a daemon on this host must not answer for the store this test names (spec 220).
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_pii"))
        .args(["run", "trace", run, "--db"])
        .arg(path)
        .env("PII_RUNTIME_DIR", dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "trace exits zero: {:?} {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn an_unfinished_node_has_a_trace_line() {
    let dir =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "trace-abandoned-").unwrap();
    let db = dir.path().join("store.redb");
    record_run(&db, "run-abandoned", false);
    let stdout = trace(dir.path(), &db, "run-abandoned");
    let leaf_line = stdout
        .lines()
        .find(|l| l.contains(" leaf "))
        .unwrap_or_else(|| panic!("a line for node leaf: {stdout}"));
    assert!(leaf_line.contains("pid=971276"), "pid in `{leaf_line}`");
    assert!(
        leaf_line.contains("abandoned"),
        "abandoned in `{leaf_line}`"
    );
}

#[test]
fn CONTROL_a_finished_node_has_one_line() {
    let dir =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "trace-finished-").unwrap();
    let db = dir.path().join("store.redb");
    record_run(&db, "run-finished", true);
    let stdout = trace(dir.path(), &db, "run-finished");
    let leaf_lines: Vec<&str> = stdout.lines().filter(|l| l.contains(" leaf ")).collect();
    assert_eq!(leaf_lines.len(), 1, "one line for leaf: {stdout}");
    assert!(
        !leaf_lines[0].contains("abandoned"),
        "a finished node is not abandoned: `{}`",
        leaf_lines[0]
    );
}

#[test]
fn the_summary_counts_the_abandoned_node() {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "trace-count-").unwrap();
    let db = dir.path().join("store.redb");
    record_run(&db, "run-count", false);
    let stdout = trace(dir.path(), &db, "run-count");
    assert!(
        stdout.lines().any(|l| l.starts_with("trace: 1 nodes ")),
        "summary counts leaf: {stdout}"
    );
}
