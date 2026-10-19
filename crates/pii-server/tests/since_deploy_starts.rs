//! Spec 192: `since_deploy` counts the process starts that wrote into a revision
//! and the runs an older process left `running`, from the run headers alone.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pii_abi::{RunEvent, RunHeader, RunId, RunStatus};
use pii_server::daemon::since_deploy::since_deploy;
use pii_store::Store;

fn scratch() -> tempfile::TempDir {
    pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "starts-").unwrap()
}

/// Seeds a run header at `revision`, written by the process started at `start`.
fn seed(store: &Store, id: &str, revision: &str, start: u64, status: RunStatus) {
    let mut h = RunHeader {
        run: RunId::from(id),
        graph: "absent-hash".into(),
        status,
        seq: 0,
        dur_ms: 0,
        revision: revision.into(),
        process_start_ms: start,
        created_ms: start,
    };
    let ev = RunEvent::Started {
        graph: "g".into(),
        input_hash: "h".into(),
    };
    store.append_event(&mut h, &ev).unwrap();
}

fn counts(store: &Store, rev: &str) -> (u64, u64) {
    let s = since_deploy(store, rev).unwrap();
    (s.starts, s.hard_deaths)
}

#[test]
fn an_older_process_leaving_a_running_run_is_a_hard_death() {
    let dir = scratch();
    let store = Store::open(&dir.path().join("s.redb")).unwrap();
    assert_eq!(counts(&store, "abc123"), (0, 0));
    seed(&store, "r1", "abc123", 100, RunStatus::Running);
    seed(&store, "r2", "abc123", 200, RunStatus::Succeeded);
    assert_eq!(counts(&store, "abc123"), (2, 1));
}

#[test]
fn the_newest_process_own_running_run_and_a_suspended_run_are_not_hard_deaths() {
    let dir = scratch();
    let store = Store::open(&dir.path().join("s.redb")).unwrap();
    seed(&store, "r1", "abc123", 100, RunStatus::Running);
    seed(&store, "r2", "abc123", 200, RunStatus::Succeeded);
    seed(&store, "r3", "abc123", 200, RunStatus::Running);
    seed(&store, "r4", "abc123", 100, RunStatus::Suspended);
    assert_eq!(counts(&store, "abc123"), (2, 1));
}

#[test]
fn other_revisions_and_unattributed_headers_move_neither_number() {
    let dir = scratch();
    let store = Store::open(&dir.path().join("s.redb")).unwrap();
    seed(&store, "r1", "abc123", 100, RunStatus::Running);
    seed(&store, "r2", "def456", 50, RunStatus::Running);
    seed(&store, "r3", "def456", 300, RunStatus::Running);
    seed(&store, "r4", "", 10, RunStatus::Running);
    assert_eq!(counts(&store, "abc123"), (1, 0));
}
