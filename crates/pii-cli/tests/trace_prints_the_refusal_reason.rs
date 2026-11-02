//! Spec 224 R2–R3: a failed node's reason is on its `pii run trace` line, cut to 80 characters, and `pii run explain`
//! states the same reason in full.

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
const REASON: &str = "criteria.unknown: 'cargo test' is not in the criteria table, so the node refused before any spawn";

/// Records one run whose only node `code` failed with `REASON` and no agent call.
fn record_run(path: &std::path::Path) {
    let store = Store::open(path).unwrap();
    let mut header = RunHeader {
        run: RunId::from("run-refused"),
        graph: "refusal-test".to_owned(),
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
        RunEvent::NodeFailed {
            node: NodeId::from("code"),
            error: REASON.to_owned(),
            retryable: false,
            attempt: 1,
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
                    error: REASON.to_owned(),
                },
                output_hash: String::new(),
            },
        )
        .unwrap();
}

/// Runs `pii run <verb> run-refused --db <db>` against a runtime dir of its own, so no daemon answers for it.
fn run_verb(verb: &str, dir: &std::path::Path, db: &std::path::Path) -> String {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_pii"))
        .args(["run", verb, "run-refused", "--db"])
        .arg(db)
        .env("PII_RUNTIME_DIR", dir)
        // State (and its durable stores) must be this test's own: the operator's are
        // held by any daemon already running on the machine.
        .env("PII_STATE_DIR", dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{verb} exits zero: {:?} {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn trace_prints_the_refusal_reason_cut_to_80() {
    assert!(REASON.chars().count() > 80, "the fixture must overflow 80");
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "refusal-cli-").unwrap();
    let db = dir.path().join("store.redb");
    record_run(&db);

    let stdout = run_verb("trace", dir.path(), &db);
    let node_line = stdout
        .lines()
        .find(|l| l.contains(" code "))
        .unwrap_or_else(|| panic!("a line for node code: {stdout}"));
    let cut: String = REASON.chars().take(80).collect();
    assert!(
        node_line.contains(&format!("fail={cut} rc=")),
        "the first 80 characters of the reason on the node's line: `{node_line}`"
    );
    assert!(!node_line.contains("fail=-"), "{node_line}");
}

#[test]
fn explain_states_the_failing_nodes_reason() {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "refusal-cli-").unwrap();
    let db = dir.path().join("store.redb");
    record_run(&db);

    let stdout = run_verb("explain", dir.path(), &db);
    assert!(
        stdout.contains(&format!("code failed: {REASON}.")),
        "explain names the failing node and its whole reason: {stdout}"
    );
}
