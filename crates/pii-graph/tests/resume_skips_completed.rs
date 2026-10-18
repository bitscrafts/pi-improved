//! Spec 186: `resume` re-dispatches only nodes with no completed journal
//! entry; `retry` re-runs ONE node with its previous output as input;
//! `abandon` closes the run as failed with the reason on the record.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use std::sync::atomic::Ordering;

use common::{Probe, chain, ctx, registry, scratch};
use pii_abi::{NodeId, PiiError, RunEvent, RunStatus, Terminal};
use pii_graph::run;
use pii_store::Store;

#[test]
fn replay_skips_completed() {
    let dir = scratch("resume-skips-");
    let store = Store::open(&dir.path().join("db.redb")).unwrap();
    let probe = Probe::default();
    let reg = registry(&probe);
    // a, b complete; c tears the run down mid-flight.
    let spec = chain([("probe", "ok"), ("probe", "ok"), ("probe", "abort")]);
    probe.broken.store(true, Ordering::SeqCst);
    let id = store.next_run_id().unwrap();
    let c = ctx(id.clone());
    *probe.root_cancel.lock() = Some(c.cancel_token().clone());
    let first = run::run(&store, &spec, &reg, &c, &id, serde_json::Value::Null);
    assert!(matches!(first, Err(PiiError::Cancelled)), "{first:?}");
    assert_eq!(
        (probe.count("a"), probe.count("b"), probe.count("c")),
        (1, 1, 1)
    );

    probe.broken.store(false, Ordering::SeqCst);
    let out = run::resume(&store, &reg, &ctx(id.clone()), &id, None).unwrap();
    assert!(matches!(out, Terminal::Success { .. }), "{out:?}");
    assert_eq!(
        (probe.count("a"), probe.count("b"), probe.count("c")),
        (1, 1, 2),
        "a and b are not re-dispatched; c runs"
    );
}

#[test]
fn retry_one_node() {
    let dir = scratch("retry-one-");
    let store = Store::open(&dir.path().join("db.redb")).unwrap();
    let probe = Probe::default();
    let reg = registry(&probe);
    let spec = chain([("probe", "ok"), ("probe", "ok"), ("probe", "ok")]);
    let id = store.next_run_id().unwrap();
    let first = run::run(
        &store,
        &spec,
        &reg,
        &ctx(id.clone()),
        &id,
        serde_json::Value::Null,
    );
    assert!(matches!(first, Ok(Terminal::Success { .. })), "{first:?}");

    let out = run::retry(&store, &reg, &ctx(id.clone()), &id, &NodeId::from("c")).unwrap();
    assert!(matches!(out, Terminal::Success { .. }), "{out:?}");
    assert_eq!(
        (probe.count("a"), probe.count("b"), probe.count("c")),
        (1, 1, 2),
        "only c runs; a and b are untouched"
    );
    let (node, nodes) = probe.seen.lock().last().cloned().unwrap();
    assert_eq!(node, "c");
    assert_eq!(
        nodes["c"]["n"], 1,
        "the previous attempt's output is available as input"
    );
}

#[test]
fn retry_of_a_failed_node_revives_the_run() {
    let dir = scratch("retry-failed-");
    let store = Store::open(&dir.path().join("db.redb")).unwrap();
    let probe = Probe::default();
    let reg = registry(&probe);
    let spec = chain([("probe", "ok"), ("probe", "ok"), ("probe", "fail")]);
    probe.broken.store(true, Ordering::SeqCst);
    let id = store.next_run_id().unwrap();
    let first = run::run(
        &store,
        &spec,
        &reg,
        &ctx(id.clone()),
        &id,
        serde_json::Value::Null,
    );
    assert!(matches!(first, Ok(Terminal::Failure { .. })), "{first:?}");

    probe.broken.store(false, Ordering::SeqCst);
    let out = run::retry(&store, &reg, &ctx(id.clone()), &id, &NodeId::from("c")).unwrap();
    assert!(matches!(out, Terminal::Success { .. }), "{out:?}");
    assert_eq!(
        (probe.count("a"), probe.count("b"), probe.count("c")),
        (1, 1, 2)
    );
    // A later plain resume replays the retry as history: nothing runs again.
    run::resume(&store, &reg, &ctx(id.clone()), &id, None).unwrap();
    assert_eq!(probe.count("c"), 2);
}

#[test]
fn retry_names_a_node_the_graph_has() {
    let dir = scratch("retry-unknown-");
    let store = Store::open(&dir.path().join("db.redb")).unwrap();
    let probe = Probe::default();
    let reg = registry(&probe);
    let spec = chain([("probe", "ok"), ("probe", "ok"), ("probe", "ok")]);
    let id = store.next_run_id().unwrap();
    run::run(
        &store,
        &spec,
        &reg,
        &ctx(id.clone()),
        &id,
        serde_json::Value::Null,
    )
    .unwrap();
    let err = run::retry(&store, &reg, &ctx(id.clone()), &id, &NodeId::from("zz")).unwrap_err();
    assert!(matches!(err, PiiError::NotFound(_)), "{err:?}");
}

#[test]
fn abandon_is_visible() {
    let dir = scratch("abandon-");
    let store = Store::open(&dir.path().join("db.redb")).unwrap();
    let probe = Probe::default();
    let reg = registry(&probe);
    let spec = chain([("probe", "ok"), ("probe", "ok"), ("probe", "abort")]);
    probe.broken.store(true, Ordering::SeqCst);
    let id = store.next_run_id().unwrap();
    let c = ctx(id.clone());
    *probe.root_cancel.lock() = Some(c.cancel_token().clone());
    let _ = run::run(&store, &spec, &reg, &c, &id, serde_json::Value::Null);
    let before = store.read_journal(&id).unwrap();

    run::abandon(&store, &ctx(id.clone()), &id, "operator gave up").unwrap();

    let header = store.get_run(&id).unwrap().unwrap();
    assert_eq!(header.status, RunStatus::Failed);
    let after = store.read_journal(&id).unwrap();
    assert_eq!(after[..before.len()], before[..], "append-only");
    assert!(
        matches!(
            &after.last().unwrap().1,
            RunEvent::Finished { terminal: Terminal::Failure { error }, .. }
                if error.contains("operator gave up")
        ),
        "{:?}",
        after.last()
    );
    let listed = store.runs_by_status(RunStatus::Failed).unwrap();
    assert!(listed.contains(&id), "listed with the successful ones");
}
