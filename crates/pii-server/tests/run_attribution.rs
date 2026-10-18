//! Spec 188: a run names the process and the deploy that produced it, and
//! `since_deploy` is read back from the store keyed by the running revision.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::sync::Arc;

use pii_abi::{RunEvent, RunHeader, RunId, RunOrigin, RunStatus};
use pii_core::{Budget, CapSet, Clock, Ctx, NullObserver, TraceCtx};
use pii_server::daemon::since_deploy::{process_origin, since_deploy};
use pii_store::Store;

struct Fixed(u64);

impl Clock for Fixed {
    fn now_ms(&self) -> u64 {
        self.0
    }
}

fn origin(revision: &str, start: u64) -> RunOrigin {
    RunOrigin {
        revision: revision.into(),
        process_start_ms: start,
    }
}

/// Runs a one-node graph through the real creation path at wall clock `now`.
fn run_real(store: &Store, now: u64) -> RunId {
    let spec = pii_graph::toml_load::load_toml(
        "name = \"t\"\nentry = \"a\"\n[[node]]\nid = \"a\"\nkind = \"transform\"\nvalue = \"1\"\n",
    )
    .unwrap();
    let run = store.next_run_id().unwrap();
    let ctx = Ctx::with_observer(
        CapSet::default(),
        Budget::unlimited(),
        Arc::new(NullObserver),
        TraceCtx {
            run: run.clone(),
            span: 0,
        },
    )
    .with_clock(Arc::new(Fixed(now)));
    pii_graph::run::run(
        store,
        &spec,
        &pii_graph::node::NodeRegistry::builtin(),
        &ctx,
        &run,
        serde_json::json!({}),
    )
    .unwrap();
    run
}

/// Writes a header as it is stored, with the given revision ("" = unattributed).
fn seed(store: &Store, id: &str, revision: &str, status: RunStatus) {
    let mut h = RunHeader {
        run: RunId::from(id),
        graph: "g".into(),
        status,
        seq: 0,
        dur_ms: 0,
        revision: revision.into(),
        process_start_ms: 0,
        created_ms: 0,
    };
    store
        .append_event(
            &mut h,
            &RunEvent::Started {
                graph: "g".into(),
                input_hash: "h".into(),
            },
        )
        .unwrap();
}

fn scratch() -> tempfile::TempDir {
    pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "attr-").unwrap()
}

fn seeded(path: &Path) {
    let store = Store::open(path).unwrap();
    seed(&store, "r1", "abc123", RunStatus::Succeeded);
    seed(&store, "r2", "abc123", RunStatus::Suspended);
    seed(&store, "r3", "def456", RunStatus::Failed);
    seed(&store, "r4", "", RunStatus::Succeeded);
}

#[test]
fn a_new_run_names_its_process_and_deploy() {
    let dir = scratch();
    let path = dir.path().join("store.redb");
    let run = {
        let store = Store::open(&path).unwrap();
        store.set_origin(origin("abc123", 4242));
        run_real(&store, 1000)
    };
    let fresh = Store::open(&path).unwrap();
    let h = fresh.get_run(&run).unwrap().unwrap();
    assert_eq!(h.revision, "abc123");
    assert_eq!(h.process_start_ms, 4242);
    assert_eq!(h.created_ms, 1000);
}

#[test]
fn since_deploy_counts_only_this_revision() {
    let dir = scratch();
    let path = dir.path().join("store.redb");
    seeded(&path);
    let s = since_deploy(&Store::open(&path).unwrap(), "abc123").unwrap();
    assert_eq!(s.revision, "abc123");
    assert_eq!(s.runs, 2);
    let by = &s.runs_by_status;
    assert_eq!(by.running + by.suspended + by.succeeded + by.failed, 2);
    assert_eq!((by.succeeded, by.suspended, by.failed), (1, 1, 0));
    assert_eq!(s.armed, 1);
    assert_eq!(s.unattributed, 1);
}

#[test]
fn since_deploy_survives_a_restart() {
    let dir = scratch();
    let path = dir.path().join("store.redb");
    seeded(&path);
    let before = since_deploy(&Store::open(&path).unwrap(), "abc123").unwrap();
    let after = since_deploy(&Store::open(&path).unwrap(), "abc123").unwrap();
    assert_eq!(before, after);
    assert_eq!((after.runs, after.unattributed), (2, 1));
}

#[test]
fn a_header_without_a_revision_is_never_this_deploy() {
    let dir = scratch();
    let path = dir.path().join("store.redb");
    seeded(&path);
    let s = since_deploy(&Store::open(&path).unwrap(), "def456").unwrap();
    assert_eq!(s.runs, 1);
    assert_eq!(s.unattributed, 1);
    // An empty running revision must not claim the unattributed rows either.
    let e = since_deploy(&Store::open(&path).unwrap(), "").unwrap();
    assert_eq!(e.runs, 0);
    assert_eq!(e.unattributed, 1);
}

#[test]
fn two_runs_of_one_process_share_their_start() {
    let dir = scratch();
    let store = Store::open(&dir.path().join("store.redb")).unwrap();
    let start = process_origin();
    assert_eq!(start, process_origin(), "built once per process");
    store.set_origin(start.clone());
    let a = run_real(&store, 10);
    let b = run_real(&store, 20);
    let (a, b) = (
        store.get_run(&a).unwrap().unwrap(),
        store.get_run(&b).unwrap().unwrap(),
    );
    assert_eq!(a.process_start_ms, b.process_start_ms);
    assert_eq!(a.process_start_ms, start.process_start_ms);
    assert_ne!((a.created_ms, b.created_ms), (0, 0));
    // A restart is a new origin: a later run carries a different start.
    store.set_origin(origin("abc123", start.process_start_ms + 1));
    let c = store.get_run(&run_real(&store, 30)).unwrap().unwrap();
    assert_ne!(c.process_start_ms, a.process_start_ms);
}
