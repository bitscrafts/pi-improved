//! Journaler write-behind tests (spec 11 J2): the bounded pending buffer
//! flushes to the store in batches, the observer stream never lags, and the
//! flush barriers (status change, snapshot, terminal) make the durable
//! journal complete at the points that matter.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::Arc;

use parking_lot::RwLock;

use pii_abi::{NodeId, RunEvent, RunId, RunStatus};
use pii_core::{Observer, RecordingObserver};
use pii_graph::blackboard::Blackboard;
use pii_graph::journal::Journaler;
use pii_store::Store;

const TMP_ROOT: &str = "/projects/pi-improved/_tmp";

fn fixture() -> (
    Store,
    RunId,
    Arc<RecordingObserver>,
    Arc<RwLock<Blackboard>>,
) {
    let dir = tempfile::Builder::new()
        .prefix("journal-test-")
        .tempdir_in(TMP_ROOT)
        .expect("tempdir under _tmp");
    let store = Store::open(&dir.path().join("db.redb")).expect("store open");
    let run = RunId::from("r000000000001");
    let obs = Arc::new(RecordingObserver::new());
    let bb = Arc::new(RwLock::new(Blackboard::new(serde_json::Value::Null)));
    (store, run, obs, bb)
}

/// The durable journal lags the observer stream: with a large batch, events
/// stay buffered until the explicit flush, but the observer sees every event
/// immediately with its assigned seq.
#[test]
fn write_behind_buffers_until_flush_but_observer_is_live() {
    let (store, run, obs, bb) = fixture();
    let jr = Journaler::with_batch(
        &store,
        Arc::clone(&obs) as Arc<dyn Observer>,
        run_header(&run),
        bb,
        1_000_000,
    );

    for i in 0..5 {
        jr.append(&RunEvent::NodeDead {
            node: NodeId::from(format!("n{i}")),
        })
        .expect("append");
    }
    // Nothing durable yet (batch is huge, no flush called).
    let journal = store.read_journal(&run).expect("read journal");
    assert_eq!(
        journal.len(),
        0,
        "nothing flushed before the explicit flush"
    );
    // Observer saw all 5 immediately, with seqs 0..5.
    let seen = obs.events();
    let journal_obs = seen
        .iter()
        .filter(|e| matches!(e, pii_core::ObsEvent::Journal { .. }))
        .count();
    assert_eq!(journal_obs, 5, "observer must see every append immediately");

    jr.flush().expect("flush");
    let journal = store.read_journal(&run).expect("read journal");
    assert_eq!(journal.len(), 5, "flush made all 5 events durable");
    assert_eq!(journal[0].0, 0, "seqs are consecutive from 0");
    assert_eq!(journal[4].0, 4, "last seq is 4");
}

/// The transaction-count probe (spec 11 J1): N buffered events flush in ONE
/// redb write transaction, not N. This is the whole point of the batching.
#[test]
fn batch_flush_uses_one_write_transaction() {
    let (store, run, obs, bb) = fixture();
    let before = store.write_txn_count();
    let jr = Journaler::with_batch(
        &store,
        obs as Arc<dyn Observer>,
        run_header(&run),
        bb,
        1_000_000,
    );
    for i in 0..10 {
        jr.append(&RunEvent::NodeDead {
            node: NodeId::from(format!("n{i}")),
        })
        .expect("append");
    }
    // 10 events buffered, zero store writes yet.
    assert_eq!(store.write_txn_count(), before, "buffering must not write");
    jr.flush().expect("flush");
    let after = store.write_txn_count();
    assert_eq!(
        after - before,
        1,
        "one flush of 10 events must be exactly ONE write transaction"
    );
    assert_eq!(store.read_journal(&run).expect("journal").len(), 10);
}

/// The batch threshold auto-flushes: with batch=2, every second append hits
/// the store.
#[test]
fn write_behind_auto_flushes_at_batch_threshold() {
    let (store, run, obs, bb) = fixture();
    let jr = Journaler::with_batch(&store, obs as Arc<dyn Observer>, run_header(&run), bb, 2);

    jr.append(&RunEvent::NodeDead {
        node: NodeId::from("a"),
    })
    .expect("append");
    assert_eq!(
        store.read_journal(&run).expect("journal").len(),
        0,
        "first append is buffered (batch=2)"
    );
    jr.append(&RunEvent::NodeDead {
        node: NodeId::from("b"),
    })
    .expect("append");
    assert_eq!(
        store.read_journal(&run).expect("journal").len(),
        2,
        "second append crossed the threshold and flushed both"
    );
}

/// A status change flushes the pending buffer first, so the persisted header
/// carries the new status (spec 11 J2 barrier).
#[test]
fn set_status_flushes_pending_events() {
    let (store, run, obs, bb) = fixture();
    let jr = Journaler::with_batch(
        &store,
        obs as Arc<dyn Observer>,
        run_header(&run),
        bb,
        1_000_000,
    );
    jr.append(&RunEvent::NodeDead {
        node: NodeId::from("a"),
    })
    .expect("append");
    // Pending, not yet durable.
    assert_eq!(store.read_journal(&run).expect("journal").len(), 0);
    jr.set_status(RunStatus::Succeeded);
    // The flush inside set_status persisted the pending event; the header
    // (read back from the store) reports the new status.
    let journal = store.read_journal(&run).expect("journal");
    assert_eq!(journal.len(), 1, "set_status flushed the pending event");
    let header = store.get_run(&run).expect("get run").expect("run exists");
    assert_eq!(
        header.status,
        RunStatus::Succeeded,
        "header carries the new status"
    );
}

/// `seq()` accounts for buffered events even before any flush.
#[test]
fn seq_advances_with_buffered_events() {
    let (store, run, obs, bb) = fixture();
    let jr = Journaler::with_batch(
        &store,
        obs as Arc<dyn Observer>,
        run_header(&run),
        bb,
        1_000_000,
    );
    assert_eq!(jr.seq(), 0);
    jr.append(&RunEvent::NodeDead {
        node: NodeId::from("a"),
    })
    .expect("append");
    assert_eq!(jr.seq(), 1, "seq reflects the buffered event");
    jr.append(&RunEvent::NodeDead {
        node: NodeId::from("b"),
    })
    .expect("append");
    assert_eq!(jr.seq(), 2);
    jr.flush().expect("flush");
    assert_eq!(jr.seq(), 2, "flush does not double-count");
}

/// `snapshot_now` flushes first, so the snapshot seq anchors a durable prefix.
#[test]
fn snapshot_now_flushes_and_anchors_durable_seq() {
    let (store, run, obs, bb) = fixture();
    let jr = Journaler::with_batch(
        &store,
        obs as Arc<dyn Observer>,
        run_header(&run),
        bb,
        1_000_000,
    );
    for i in 0..3 {
        jr.append(&RunEvent::NodeDead {
            node: NodeId::from(format!("n{i}")),
        })
        .expect("append");
    }
    let seq = jr.snapshot_now().expect("snapshot");
    assert_eq!(seq, 2, "snapshot anchors at the last flushed event seq");
    assert_eq!(
        store.read_journal(&run).expect("journal").len(),
        3,
        "snapshot_now flushed the pending events first"
    );
}

fn run_header(run: &RunId) -> pii_abi::RunHeader {
    pii_abi::RunHeader {
        run: run.clone(),
        graph: String::from("test"),
        status: RunStatus::Running,
        seq: 0,
        dur_ms: 0,
    }
}
