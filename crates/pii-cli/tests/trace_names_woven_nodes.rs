//! Spec 219 R4: `pii run trace <run_id>` prints each woven node with its origin, so the graph that is read is the graph
//! that ran.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
// reason: the test runs the built `pii` binary and reads its stdout.
#![allow(clippy::disallowed_methods)]

use pii_abi::{NodeId, PortName, RunEvent, RunHeader, RunId, RunStatus, Terminal};
use pii_store::Store;

/// Records a run whose woven guard node ran, with the origin the weave stamped on it.
fn record_run(path: &std::path::Path) {
    let store = Store::open(path).unwrap();
    let mut header = RunHeader {
        run: RunId::from("run-woven"),
        graph: "guarded".to_owned(),
        status: RunStatus::Running,
        seq: 0,
        dur_ms: 0,
        revision: String::new(),
        process_start_ms: 1_000,
        created_ms: 1_000,
    };
    let events = [
        RunEvent::NodeObserved {
            node: NodeId::from("guard.check"),
            kind: "woven".to_owned(),
            fields: serde_json::json!({ "woven-from": "podman-guard#check" }),
        },
        RunEvent::NodeStarted {
            node: NodeId::from("guard.check"),
            at_ms: 10,
        },
        RunEvent::NodeFinished {
            node: NodeId::from("guard.check"),
            port: PortName::from("ok"),
            value: serde_json::json!({}),
            value_hash: String::new(),
            at_ms: 10,
            dur_ms: 3,
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
fn run_trace_names_the_woven_origin() {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "trace-woven-").unwrap();
    let db = dir.path().join("store.redb");
    record_run(&db);

    // Its own runtime dir: a daemon on this host must not answer for the store this test names (spec 220).
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_pii"))
        .args(["run", "trace", "run-woven", "--db"])
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
        .find(|l| l.contains(" guard.check "))
        .unwrap_or_else(|| panic!("a line for woven node guard.check: {stdout}"));
    assert!(
        node_line.contains("woven-from: podman-guard#check"),
        "origin on the woven node's line: `{node_line}`"
    );
}
