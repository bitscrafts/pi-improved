//! Spec 217 R9: an orphan is a `coder.spawned` whose pid has no `coder.exited` in the same run, counted once the run
//! reaches a terminal state. The count is derived from the journal, so it survives a restart.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pii_abi::{NodeId, RunEvent, RunHeader, RunId, RunStatus, Terminal};
use pii_graph::agent_calls::store_totals;
use pii_store::Store;

const PROCESS_START_MS: u64 = 1_000;

fn observed(kind: &str, fields: serde_json::Value) -> RunEvent {
    RunEvent::NodeObserved {
        node: NodeId::from("code"),
        kind: kind.to_owned(),
        fields,
    }
}

#[test]
fn orphan_is_countable() {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "orphan-store-").unwrap();
    let store = Store::open(&dir.path().join("store.redb")).unwrap();
    let mut header = RunHeader {
        run: RunId::from("run-orphan"),
        graph: "orphan-test".to_owned(),
        status: RunStatus::Running,
        seq: 0,
        dur_ms: 0,
        revision: String::new(),
        process_start_ms: PROCESS_START_MS,
        created_ms: PROCESS_START_MS,
    };
    // A spawn with no exit: the agent's process is gone from the journal's point of view.
    store
        .append_event(
            &mut header,
            &observed(
                "coder.spawned",
                serde_json::json!({ "pid": 4242, "agent": "pi", "argv_digest": "00" }),
            ),
        )
        .unwrap();

    let running = store_totals(&store, PROCESS_START_MS).unwrap();
    assert_eq!(
        running.orphans_total, 0,
        "a running run is not an orphan yet"
    );
    assert_eq!(
        running.agents_in_flight, 1,
        "the unmatched spawn is in flight"
    );

    header.status = RunStatus::Failed;
    store
        .append_event(
            &mut header,
            &RunEvent::Finished {
                terminal: Terminal::Failure {
                    error: "stopped".to_owned(),
                },
                output_hash: String::new(),
            },
        )
        .unwrap();

    let after = store_totals(&store, PROCESS_START_MS).unwrap();
    assert_eq!(
        after.orphans_total,
        running.orphans_total + 1,
        "reaching a terminal state counts the orphan exactly once"
    );
    assert_eq!(
        after.orphans_since_start, 1,
        "the orphan belongs to this process"
    );
    assert_eq!(
        after.agents_in_flight, 0,
        "a terminal run has nothing in flight"
    );
}
