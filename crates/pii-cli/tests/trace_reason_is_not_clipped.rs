//! Spec 231 R1/R4: `pii run trace <run> --reason <node>` prints the node's failure string whole, byte for byte, while the
//! human trace line keeps its 80-character cut.

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
const RUN: &str = "run-long-reason";
const TAIL: &str = ": permission denied";

/// A 400-character reason that ends the way a runtime refusal does.
fn long_reason() -> String {
    format!("{}{TAIL}", "x".repeat(400 - TAIL.len()))
}

/// Records one run whose only node `code` failed with `reason` and no agent call.
fn record_run(path: &std::path::Path, reason: &str) {
    let store = Store::open(path).unwrap();
    let mut header = RunHeader {
        run: RunId::from(RUN),
        graph: "reason-test".to_owned(),
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
            error: reason.to_owned(),
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
                    error: reason.to_owned(),
                },
                output_hash: String::new(),
            },
        )
        .unwrap();
}

/// Runs `pii run <args>` against a runtime dir of its own, so no daemon answers for it. Returns stdout.
fn pii_run(dir: &std::path::Path, args: &[&str]) -> String {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_pii"))
        .args(["run"])
        .args(args)
        .env("PII_RUNTIME_DIR", dir)
        // State (and its durable stores) must be this test's own: the operator's are
        // held by any daemon already running on the machine.
        .env("PII_STATE_DIR", dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "pii run {args:?} exits zero: {:?} {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn reason_flag_prints_the_whole_reason() {
    let reason = long_reason();
    assert_eq!(reason.chars().count(), 400, "the fixture is 400 characters");
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "reason-cli-").unwrap();
    let db = dir.path().join("store.redb");
    record_run(&db, &reason);
    let db_str = db.display().to_string();

    let stdout = pii_run(
        dir.path(),
        &["trace", RUN, "--db", &db_str, "--reason", "code"],
    );
    assert_eq!(
        stdout,
        format!("{reason}\n"),
        "the node's reason, byte for byte"
    );
    assert_ne!(
        stdout.trim_end().chars().count(),
        80,
        "not the 80-character cut"
    );
}

#[test]
fn human_trace_line_keeps_the_80_character_cut() {
    let reason = long_reason();
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "reason-cli-").unwrap();
    let db = dir.path().join("store.redb");
    record_run(&db, &reason);
    let db_str = db.display().to_string();

    let stdout = pii_run(dir.path(), &["trace", RUN, "--db", &db_str]);
    let node_line = stdout
        .lines()
        .find(|l| l.contains(" code "))
        .unwrap_or_else(|| panic!("a line for node code: {stdout}"));
    let cut: String = reason.chars().take(80).collect();
    assert!(
        node_line.contains(&format!("fail={cut} rc=")),
        "the display line still cuts at 80: `{node_line}`"
    );
    assert!(
        !node_line.contains(TAIL),
        "the cut hides the tail on the display line: `{node_line}`"
    );
}
