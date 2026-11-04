//! Spec 376 R3: a run that is `Running` or `Suspended` is kept by a sweep however old it is.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use pii_abi::{NodeId, RunEvent, RunHeader, RunId, RunStatus};
use pii_store::Store;
use pii_store::retention::RetentionPolicy;

const DAY_MS: u64 = 86_400_000;
const NOW: u64 = 400 * DAY_MS;

/// Every seeded run is 40 days old, so each one is inside the default policy's reach — the point of
/// this test is that age alone does not decide, status does.
fn write_run(store: &Store, run: &str, status: RunStatus) {
    let mut h = RunHeader {
        run: RunId::from(run),
        graph: String::from("g"),
        status,
        seq: 0,
        dur_ms: 0,
        revision: String::new(),
        process_start_ms: 0,
        created_ms: NOW - 40 * DAY_MS,
    };
    store
        .append_event(
            &mut h,
            &RunEvent::Started {
                graph: String::from("g"),
                input_hash: String::from("h"),
            },
        )
        .unwrap();
    store
        .append_event(
            &mut h,
            &RunEvent::NodeStarted {
                node: NodeId::from("n0"),
                at_ms: 0,
            },
        )
        .unwrap();
}

#[test]
fn a_running_or_suspended_run_survives_a_sweep_however_old() {
    let dir =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "retention-live-").unwrap();
    let store = Store::open(&dir.path().join("store.redb")).unwrap();
    write_run(&store, "running", RunStatus::Running);
    write_run(&store, "suspended", RunStatus::Suspended);
    write_run(&store, "done", RunStatus::Succeeded);

    let removed = store
        .sweep_retention(NOW, &RetentionPolicy::default())
        .unwrap();

    // Only the finished run goes: 2 journal rows + 1 header row.
    assert_eq!(removed, 3);
    for live in ["running", "suspended"] {
        let id = RunId::from(live);
        assert!(store.get_run(&id).unwrap().is_some(), "{live} was dropped");
        assert_eq!(
            store.read_journal(&id).unwrap().len(),
            2,
            "{live} lost events"
        );
    }
    assert!(store.get_run(&RunId::from("done")).unwrap().is_none());
}
