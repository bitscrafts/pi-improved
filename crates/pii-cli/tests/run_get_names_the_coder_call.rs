//! Spec 221 R4: `pii run get <run_id>` prints, for each coder node, `spawned=<pid|none> exited=<rc|none>
//! refused=<reason|none>`, built from the journal. With no daemon reachable it reads the store named by `--db`.

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

/// Records one run `run` whose journal holds `events` and whose terminal status is `status`.
fn record_run(store: &Store, run: &str, events: &[RunEvent], status: RunStatus) {
    let mut header = RunHeader {
        run: RunId::from(run),
        graph: "get-test".to_owned(),
        status: RunStatus::Running,
        seq: 0,
        dur_ms: 0,
        revision: String::new(),
        process_start_ms: PROCESS_START_MS,
        created_ms: PROCESS_START_MS,
    };
    for event in events {
        store.append_event(&mut header, event).unwrap();
    }
    header.status = status;
    store
        .append_event(
            &mut header,
            &RunEvent::Finished {
                terminal: Terminal::Success {
                    output: serde_json::json!("done"),
                },
                output_hash: String::new(),
            },
        )
        .unwrap();
}

/// A coder node's observation in the journal, as the engine records it.
fn observe(kind: &str, fields: serde_json::Value) -> RunEvent {
    RunEvent::NodeObserved {
        node: NodeId::from("code"),
        kind: kind.to_owned(),
        fields,
    }
}

/// `pii run get <run>` with no daemon: the store at `db` answers, and stdout carries the coder line.
fn get(db: &std::path::Path, runtime: &std::path::Path, run: &str) -> (bool, String) {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_pii"))
        .args(["run", "get", run, "--db"])
        .arg(db)
        .env("PII_RUNTIME_DIR", runtime)
        .output()
        .unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

#[test]
fn run_get_names_the_coder_call() {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "get-cli-").unwrap();
    let db = dir.path().join("store.redb");
    let store = Store::open(&db).unwrap();
    record_run(
        &store,
        "run-spawned",
        &[
            RunEvent::NodeStarted {
                node: NodeId::from("code"),
                at_ms: 10,
            },
            observe(
                "coder.spawned",
                serde_json::json!({ "pid": 4242, "agent": "pi", "argv_digest": "00" }),
            ),
            observe(
                "coder.exited",
                serde_json::json!({ "pid": 4242, "rc": 0, "criteria_run": 1, "criteria_passed": 1 }),
            ),
        ],
        RunStatus::Succeeded,
    );
    record_run(
        &store,
        "run-refused",
        &[observe(
            "worker.start.refused",
            serde_json::json!({ "reason": "no credentials in env_pass", "model": "haiku-5-5" }),
        )],
        RunStatus::Refused,
    );
    drop(store);

    // Its own runtime dir: no daemon answers, so the store named by --db is the source (spec 220).
    let (ok, stdout) = get(&db, dir.path(), "run-spawned");
    assert!(ok, "get exits zero: {stdout}");
    assert!(
        stdout
            .lines()
            .any(|l| l == "code spawned=4242 exited=0 refused=none"),
        "the coder line names the call: {stdout}"
    );

    let (ok, stdout) = get(&db, dir.path(), "run-refused");
    assert!(ok, "get exits zero: {stdout}");
    assert!(
        stdout
            .lines()
            .any(|l| { l == "code spawned=none exited=none refused=no credentials in env_pass" }),
        "the coder line names the refusal: {stdout}"
    );
}
