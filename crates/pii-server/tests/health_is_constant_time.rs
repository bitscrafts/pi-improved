//! Spec 370: `sys.status` reads one maintained record instead of replaying every journal. The record must give the
//! same numbers as the replay it replaces (R4), stay equal to the replay after every run change (R1), and be rebuilt
//! once, visibly, when it is absent (R2).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use pii_abi::{NodeId, RunEvent, RunHeader, RunId, RunStatus, Terminal};
use pii_graph::agent_calls::store_totals;
use pii_server::daemon::since_deploy::since_deploy;
use pii_server::daemon::sys_status::{SysStatus, observe_append, sys_status_for_health};
use pii_store::Store;

const CURRENT_START_MS: u64 = 2_000;
const REVISION: &str = "rev-a";

fn observed(kind: &str, fields: serde_json::Value) -> RunEvent {
    RunEvent::NodeObserved {
        node: NodeId::from("code"),
        kind: kind.to_owned(),
        fields,
    }
}

fn header(run: &str, revision: &str, process_start_ms: u64) -> RunHeader {
    RunHeader {
        run: RunId::from(run),
        graph: "health-fixture".to_owned(),
        status: RunStatus::Running,
        seq: 0,
        dur_ms: 0,
        revision: revision.to_owned(),
        process_start_ms,
        created_ms: process_start_ms,
    }
}

fn finished(error: &str) -> RunEvent {
    RunEvent::Finished {
        terminal: Terminal::Failure {
            error: error.to_owned(),
        },
        output_hash: String::new(),
    }
}

/// Applies the fixture to `store` one event at a time, calling `after_each` after every append. It covers every kind
/// of run the totals count: a clean spawn/exit, a criteria failure, in-flight agents, an orphan under the current
/// process, a run from an older process, a refused start, and an unattributed header.
fn play(store: &Store, mut after_each: impl FnMut(&Store)) {
    let spawn = |pid: i64| {
        observed(
            "coder.spawned",
            serde_json::json!({ "pid": pid, "agent": "pi" }),
        )
    };
    let exit = |pid: i64, failed: i64| {
        observed(
            "coder.exited",
            serde_json::json!({ "pid": pid, "criteria_failed": failed }),
        )
    };
    let mut run_a = header("run-a", REVISION, 1_000);
    let mut run_b = header("run-b", REVISION, 1_000);
    let mut run_c = header("run-c", "rev-b", CURRENT_START_MS);
    let mut run_d = header("run-d", "", 500);
    let mut run_e = header("run-e", REVISION, 1_500);

    let mut step = |h: &mut RunHeader, status: RunStatus, event: RunEvent| {
        h.status = status;
        store.append_event(h, &event).unwrap();
        after_each(store);
    };
    // run-a: spawn, exit with a failed criterion, succeed.
    step(&mut run_a, RunStatus::Running, spawn(1));
    step(&mut run_a, RunStatus::Running, exit(1, 1));
    step(&mut run_a, RunStatus::Succeeded, finished("ok"));
    // run-b: in flight, still running.
    step(&mut run_b, RunStatus::Running, spawn(2));
    // run-c: an orphan under the current process once it fails.
    step(&mut run_c, RunStatus::Running, spawn(3));
    step(&mut run_c, RunStatus::Failed, finished("stopped"));
    // run-d: refused before any spawn, with no revision.
    step(
        &mut run_d,
        RunStatus::Refused,
        observed("worker.start.refused", serde_json::json!({})),
    );
    // run-e: a running run from an older process than the newest start.
    step(&mut run_e, RunStatus::Running, spawn(4));
}

#[test]
fn record_equals_replay_after_every_run_change() {
    let dir =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "health-record-").unwrap();
    let store = Store::open(&dir.path().join("store.redb")).unwrap();
    store.set_append_observer(observe_append);
    let mut steps = 0;
    play(&store, |s| {
        steps += 1;
        let record = SysStatus::read(s).unwrap().expect("maintained record");
        let replay = (
            store_totals(s, CURRENT_START_MS).unwrap(),
            since_deploy(s, REVISION).unwrap(),
        );
        assert_eq!(
            record.counts(CURRENT_START_MS, REVISION),
            replay,
            "after append {steps}"
        );
    });
    assert_eq!(steps, 8, "every append was checked");
}

#[test]
fn record_numbers_match_the_replay_they_replace() {
    let dir =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "health-replay-").unwrap();
    let store = Store::open(&dir.path().join("store.redb")).unwrap();
    store.set_append_observer(observe_append);
    play(&store, |_| {});
    let record = SysStatus::read(&store).unwrap().expect("maintained record");
    let (totals, deploy) = record.counts(CURRENT_START_MS, REVISION);
    assert_eq!(totals.dispatches_total, 4, "four spawns across the runs");
    assert_eq!(
        totals.criteria_failures_total, 1,
        "one exit with a failed criterion"
    );
    assert_eq!(
        totals.refusals_total, 1,
        "run-d's worker.start.refused observation, counted as the replay counts it"
    );
    assert_eq!(totals.orphans_total, 1, "run-c spawned and failed unexited");
    assert_eq!(
        totals.orphans_since_start, 1,
        "run-c is the current process's"
    );
    assert_eq!(
        totals.agents_in_flight, 2,
        "run-b and run-e are still in flight"
    );
    assert_eq!(deploy.runs, 3, "run-a, run-b and run-e carry this revision");
    assert_eq!(deploy.starts, 2, "two process starts wrote this revision");
    assert_eq!(
        deploy.hard_deaths, 1,
        "run-b is running under an older start"
    );
    assert_eq!(deploy.unattributed, 1, "run-d has no revision");
}

#[test]
fn absent_record_is_rebuilt_once_and_says_so() {
    let dir =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "health-rebuild-").unwrap();
    let store = Store::open(&dir.path().join("store.redb")).unwrap();
    // No observer: the appends leave no record, as in a store written before this spec.
    play(&store, |_| {});
    assert!(SysStatus::read(&store).unwrap().is_none(), "no record yet");

    let (first, rebuilt) = sys_status_for_health(&store, CURRENT_START_MS, REVISION).unwrap();
    assert!(rebuilt, "the first call reports the rebuild");
    assert_eq!(
        first.counts(CURRENT_START_MS, REVISION),
        (
            store_totals(&store, CURRENT_START_MS).unwrap(),
            since_deploy(&store, REVISION).unwrap(),
        ),
        "the rebuilt record equals the replay"
    );

    let (_, rebuilt_again) = sys_status_for_health(&store, CURRENT_START_MS, REVISION).unwrap();
    assert!(!rebuilt_again, "the second call reads the record");
}
