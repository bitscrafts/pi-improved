//! Journal appending for a run (spec F4.10, pass-3 §4.8).
//!
//! Every event is appended through exactly one redb write transaction —
//! opened and committed inside [`pii_store::Store::append_event`]; no
//! transaction is ever held across a node dispatch. The journaler owns the
//! run header (its `seq` is the single sequence counter) and snapshots the
//! blackboard every [`SNAPSHOT_EVERY`] journal events and unconditionally on
//! suspend.
//!
//! Resume (Phase 1) replays the whole journal and does **not** read
//! snapshots; snapshots remain a crash-recovery optimisation for later
//! phases and are still written on this cadence.

use std::sync::Arc;

use parking_lot::{Mutex, RwLock};

use pii_abi::{PiiError, RunEvent, RunHeader, RunId, RunStatus};
use pii_core::ObsEvent;
use pii_store::Store;

use crate::blackboard::Blackboard;

/// A blackboard snapshot is taken every time a journal event lands on a seq
/// that is a nonzero multiple of this (and unconditionally on suspend).
pub const SNAPSHOT_EVERY: u64 = 32;

struct JournalState {
    header: RunHeader,
    /// Seq of the most recently appended event (the snapshot anchor).
    last_seq: u64,
}

/// Appends run events to the store, advancing the header and driving the
/// snapshot cadence. Shared `&`-only with worker threads.
///
/// Interior mutability is a `parking_lot::Mutex` over the header state (never
/// a redb transaction).
pub struct Journaler<'a> {
    store: &'a Store,
    obs: Arc<dyn pii_core::Observer>,
    state: Mutex<JournalState>,
    bb: Arc<RwLock<Blackboard>>,
}

impl<'a> Journaler<'a> {
    /// A journaler for a fresh run whose header starts at seq 0, `Running`.
    #[must_use]
    pub fn fresh(
        store: &'a Store,
        obs: Arc<dyn pii_core::Observer>,
        run: RunId,
        graph_hash: String,
        bb: Arc<RwLock<Blackboard>>,
    ) -> Self {
        Self {
            store,
            obs,
            state: Mutex::new(JournalState {
                header: RunHeader {
                    run,
                    graph: graph_hash,
                    status: RunStatus::Running,
                    seq: 0,
                    dur_ms: 0,
                },
                last_seq: 0,
            }),
            bb,
        }
    }

    /// A journaler that adopts a persisted header (resume continues the seq).
    #[must_use]
    pub fn adopt(
        store: &'a Store,
        obs: Arc<dyn pii_core::Observer>,
        header: RunHeader,
        bb: Arc<RwLock<Blackboard>>,
    ) -> Self {
        let last_seq = header.seq.saturating_sub(1);
        Self {
            store,
            obs,
            state: Mutex::new(JournalState { header, last_seq }),
            bb,
        }
    }

    /// The store handle (for `put_resume_key` and content-addressed bodies).
    #[must_use]
    pub fn store(&self) -> &'a Store {
        self.store
    }

    /// The next sequence number to be written.
    #[must_use]
    pub fn seq(&self) -> u64 {
        self.state.lock().header.seq
    }

    /// Sets the header status (call before appending the `Finished` event so
    /// the run is listed under its final status).
    pub fn set_status(&self, status: RunStatus) {
        self.state.lock().header.status = status;
    }

    /// Records the total run duration on the header.
    pub fn set_dur_ms(&self, ms: u32) {
        self.state.lock().header.dur_ms = ms;
    }

    /// Appends `event` in one write transaction, notifies the observer with
    /// the assigned seq, and snapshots the blackboard when the event's seq is
    /// a nonzero multiple of [`SNAPSHOT_EVERY`]. Returns the seq written.
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] on a redb or serialisation failure.
    pub fn append(&self, event: &RunEvent) -> Result<u64, PiiError> {
        let seq = {
            let mut st = self.state.lock();
            let s = st.header.seq;
            self.store.append_event(&mut st.header, event)?;
            st.last_seq = s;
            s
        };
        self.obs.event(&ObsEvent::Journal {
            seq,
            event: event.clone(),
        });
        if seq > 0 && seq.is_multiple_of(SNAPSHOT_EVERY) {
            Self::snapshot_locked(&self.bb, self.store, &self.state.lock().header.run, seq)?;
        }
        Ok(seq)
    }

    /// Snapshots the blackboard at the most recently appended seq (used
    /// unconditionally on suspend).
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] on a redb or serialisation failure.
    pub fn snapshot_now(&self) -> Result<u64, PiiError> {
        let (seq, run) = {
            let st = self.state.lock();
            (st.last_seq, st.header.run.clone())
        };
        Self::snapshot_locked(&self.bb, self.store, &run, seq)?;
        Ok(seq)
    }

    fn snapshot_locked(
        bb: &RwLock<Blackboard>,
        store: &Store,
        run: &RunId,
        seq: u64,
    ) -> Result<(), PiiError> {
        let bytes = bb.read().snapshot_bytes()?;
        store.put_snapshot(run, seq, &bytes)
    }
}
