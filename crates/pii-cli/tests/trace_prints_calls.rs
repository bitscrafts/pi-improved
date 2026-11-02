//! Spec 270 R3/R4 (i)-(k): `pii run trace <run>` prints one `call` line per `graph.call` observation, in the shape
//! `pii graph explain` prints, and a nested call names its parent. A run with no call prints no `call` line.

#![allow(
    non_snake_case,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
// reason: the test runs the built `pii` binary and reads its stdout.
#![allow(clippy::disallowed_methods)]

use pii_abi::{NodeId, PortName, RunEvent, RunHeader, RunId, RunStatus, Terminal};
use pii_store::Store;

/// The full digest the call record carries: 64 hex digits whose first twelve are `0123456789ab`.
const SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// Records a run with the given extra events before its one node's start and finish.
fn record_run(path: &std::path::Path, run: &str, extra: Vec<RunEvent>) {
    let store = Store::open(path).unwrap();
    let mut header = RunHeader {
        run: RunId::from(run),
        graph: "docs-host".to_owned(),
        status: RunStatus::Running,
        seq: 0,
        dur_ms: 0,
        revision: String::new(),
        process_start_ms: 1_000,
        created_ms: 1_000,
    };
    let mut events = extra;
    events.push(RunEvent::NodeStarted {
        node: NodeId::from("done"),
        at_ms: 10,
    });
    events.push(RunEvent::NodeFinished {
        node: NodeId::from("done"),
        port: PortName::from("ok"),
        value: serde_json::json!({}),
        value_hash: String::new(),
        at_ms: 10,
        dur_ms: 3,
    });
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

fn call_event(node: &str, parent: Option<&str>) -> RunEvent {
    let mut fields = serde_json::json!({
        "graph": "note",
        "sha256": SHA,
        "caps": [],
        "exit": "fmt",
        "input_keys": ["title"],
    });
    if let Some(p) = parent {
        fields["parent"] = serde_json::json!(p);
    }
    RunEvent::NodeObserved {
        node: NodeId::from(node),
        kind: "graph.call".to_owned(),
        fields,
    }
}

/// Runs `pii run trace <run>` against the store at `db` and returns its stdout.
fn trace(dir: &std::path::Path, db: &std::path::Path, run: &str) -> String {
    // Its own runtime dir: a daemon on this host must not answer for the store this test names (spec 220).
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_pii"))
        .args(["run", "trace", run, "--db"])
        .arg(db)
        .env("PII_RUNTIME_DIR", dir)
        // State (and its durable stores) must be this test's own: the operator's are
        // held by any daemon already running on the machine.
        .env("PII_STATE_DIR", dir)
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
fn trace_prints_the_call_line() {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "trace-calls-").unwrap();
    let db = dir.path().join("store.redb");
    record_run(&db, "run-call", vec![call_event("docs__in", None)]);
    let stdout = trace(dir.path(), &db, "run-call");
    assert!(
        stdout
            .lines()
            .any(|l| l
                == "call docs -> graph note sha256 0123456789ab caps [] needs [none] exit fmt"),
        "the exact call line: {stdout}"
    );
}

#[test]
fn trace_prints_the_parent_of_a_nested_call() {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "trace-calls-").unwrap();
    let db = dir.path().join("store.redb");
    record_run(
        &db,
        "run-nested",
        vec![
            call_event("docs__in", None),
            call_event("docs__inner__in", Some("docs")),
        ],
    );
    let stdout = trace(dir.path(), &db, "run-nested");
    assert!(
        stdout.lines().any(|l| l == "call docs__inner -> graph note sha256 0123456789ab caps [] needs [none] exit fmt (in docs)"),
        "the nested call line names its parent: {stdout}"
    );
}

#[test]
fn CONTROL_trace_prints_no_call_line_without_a_call() {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "trace-calls-").unwrap();
    let db = dir.path().join("store.redb");
    record_run(&db, "run-plain", Vec::new());
    let stdout = trace(dir.path(), &db, "run-plain");
    assert!(
        !stdout.lines().any(|l| l.starts_with("call ")),
        "no call line for a run with no call: {stdout}"
    );
}
