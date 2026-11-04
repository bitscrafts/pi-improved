//! Spec 376 R1/R2/R5: a sweep drops what the policy expires, never more than its cap per call, and reports it.

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
const NOW: u64 = 100 * DAY_MS;

fn header(run: &str, status: RunStatus, created_ms: u64) -> RunHeader {
    RunHeader {
        run: RunId::from(run),
        graph: String::from("g"),
        status,
        seq: 0,
        dur_ms: 0,
        revision: String::new(),
        process_start_ms: 0,
        created_ms,
    }
}

fn node_started(node: &str) -> RunEvent {
    RunEvent::NodeStarted {
        node: NodeId::from(node),
        at_ms: 0,
    }
}

/// Writes a run with one `Started` event followed by `nodes` node-call events.
fn write_run(store: &Store, run: &str, status: RunStatus, created_ms: u64, nodes: usize) {
    let mut h = header(run, status, created_ms);
    store
        .append_event(
            &mut h,
            &RunEvent::Started {
                graph: String::from("g"),
                input_hash: String::from("h"),
            },
        )
        .unwrap();
    for i in 0..nodes {
        store
            .append_event(&mut h, &node_started(&format!("n{i}")))
            .unwrap();
    }
}

fn policy(max_rows: usize) -> RetentionPolicy {
    RetentionPolicy {
        max_rows,
        ..RetentionPolicy::default()
    }
}

#[test]
fn an_old_finished_run_is_dropped_and_a_recent_one_is_kept() {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "retention-").unwrap();
    let store = Store::open(&dir.path().join("store.redb")).unwrap();
    write_run(&store, "old", RunStatus::Succeeded, NOW - 40 * DAY_MS, 1);
    write_run(&store, "fresh", RunStatus::Succeeded, NOW, 1);

    // old: 2 journal rows + 1 header row; fresh: untouched.
    let removed = store
        .sweep_retention(NOW, &RetentionPolicy::default())
        .unwrap();

    assert_eq!(removed, 3);
    assert!(store.get_run(&RunId::from("old")).unwrap().is_none());
    assert_eq!(store.read_journal(&RunId::from("old")).unwrap().len(), 0);
    assert!(store.get_run(&RunId::from("fresh")).unwrap().is_some());
    assert_eq!(store.read_journal(&RunId::from("fresh")).unwrap().len(), 2);
}

#[test]
fn node_call_events_expire_at_seven_days_and_the_run_at_thirty() {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "retention-").unwrap();
    let store = Store::open(&dir.path().join("store.redb")).unwrap();
    // Ten days old: past the 7-day node-call age, inside the 30-day run age.
    write_run(
        &store,
        "ten-days",
        RunStatus::Succeeded,
        NOW - 10 * DAY_MS,
        2,
    );

    let removed = store
        .sweep_retention(NOW, &RetentionPolicy::default())
        .unwrap();

    assert_eq!(removed, 2, "only the two NodeStarted rows go");
    let left = store.read_journal(&RunId::from("ten-days")).unwrap();
    assert_eq!(left.len(), 1);
    assert!(matches!(left[0].1, RunEvent::Started { .. }));
    assert!(store.get_run(&RunId::from("ten-days")).unwrap().is_some());
}

#[test]
fn a_sweep_removes_at_most_its_cap_and_repeated_sweeps_finish_the_job() {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "retention-").unwrap();
    let store = Store::open(&dir.path().join("store.redb")).unwrap();
    // 1 Started + 24 nodes = 25 journal rows, plus 1 header row = 26 eligible rows.
    write_run(&store, "big", RunStatus::Succeeded, NOW - 40 * DAY_MS, 24);

    let first = store.sweep_retention(NOW, &policy(10)).unwrap();
    assert_eq!(first, 10, "the cap is honoured, not just the age");
    assert!(store.get_run(&RunId::from("big")).unwrap().is_some());

    let mut total = first;
    loop {
        let n = store.sweep_retention(NOW, &policy(10)).unwrap();
        if n == 0 {
            break;
        }
        assert!(n <= 10);
        total += n;
    }
    assert_eq!(total, 26);
    assert!(store.get_run(&RunId::from("big")).unwrap().is_none());
}

#[test]
fn a_sweep_writes_one_event_naming_the_count() {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "retention-").unwrap();
    let store = Store::open(&dir.path().join("store.redb")).unwrap();
    let recorder = std::sync::Arc::new(pii_observe::Recorder::default());
    store.set_observer(recorder.clone());
    write_run(&store, "old", RunStatus::Succeeded, NOW - 40 * DAY_MS, 1);

    let removed = store
        .sweep_retention(NOW, &RetentionPolicy::default())
        .unwrap();

    let events = recorder.events();
    let sweeps: Vec<_> = events
        .iter()
        .filter(|e| e.kind == "store.retention.swept")
        .collect();
    assert_eq!(sweeps.len(), 1);
    assert_eq!(
        sweeps[0].fields.get("removed"),
        Some(&serde_json::json!(removed))
    );
}

#[test]
fn a_sweep_of_an_empty_store_removes_nothing() {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "retention-").unwrap();
    let store = Store::open(&dir.path().join("store.redb")).unwrap();
    assert_eq!(
        store
            .sweep_retention(NOW, &RetentionPolicy::default())
            .unwrap(),
        0
    );
}
