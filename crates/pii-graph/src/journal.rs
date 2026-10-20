//! Journal appending for a run (spec F4.10, pass-3 §4.8; hardened spec 11).
//!
//! Every event is appended through [`pii_store::Store::append_events`] —
//! events accumulate in a bounded pending buffer and are flushed in ONE redb
//! write transaction per batch (spec 11 J2: the pre-hardening code opened one
//! write transaction per event, which was the dominant RSS slope driver under
//! bursty node chatter). No transaction is ever held across a node dispatch.
//! The journaler owns the run header (its `seq` is the single sequence
//! counter) and snapshots the blackboard every [`SNAPSHOT_EVERY`] flushed
//! events and unconditionally on suspend.
//!
//! Write-behind semantics (spec 11): `append` assigns the seq eagerly (the
//! store writes events at exactly those seqs on flush, since both derive from
//! the same `header.seq` counter), notifies the observer IMMEDIATELY — the
//! observer stream is the LIVE view (TUI / subscribe bus) and must not lag —
//! and buffers the event for the durable store journal. The durable view may
//! lag by up to the batch threshold. Explicit flush barriers run at the
//! points where durability matters: a status change ([`Journaler::set_status`]
//! flushes first, so the persisted header carries the new status), a snapshot
//! ([`Journaler::snapshot_now`], the suspend path), and the terminal
//! `Finished` event (the `finish` path in `run.rs` calls [`Journaler::flush`]
//! after appending it, so a completed run's journal is fully durable the
//! moment `run`/`resume` returns). A crash mid-run loses only unflushed
//! events — the journal still ends at a valid contiguous flushed prefix and
//! the persisted header still reports `Running`, so boot recovery is
//! unchanged. The `Cancelled` path deliberately does NOT flush (spec 11
//! guardrail: durability does not matter for a run that never finishes).

use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::{Mutex, RwLock};

use pii_abi::{PiiError, RunEvent, RunHeader, RunId, RunStatus};
use pii_core::ObsEvent;
use pii_store::Store;

use crate::blackboard::Blackboard;

/// A blackboard snapshot is taken every time a flushed journal event lands on
/// a seq that is a nonzero multiple of this (and unconditionally on suspend).
pub const SNAPSHOT_EVERY: u64 = 32;

/// Default write-behind batch threshold: pending events are flushed to the
/// store once this many have accumulated (spec 11 J2).
pub const DEFAULT_BATCH: usize = 16;

struct JournalState {
    header: RunHeader,
    /// Seq of the most recently flushed event (the snapshot anchor).
    last_seq: u64,
    /// Events buffered but not yet flushed to the store (write-behind).
    pending: Vec<RunEvent>,
    /// True when `header` has fields changed (`status`/`dur_ms`) since the
    /// last flush that `pending` alone would not capture — guards against
    /// `flush`'s empty-pending fast path silently dropping a header-only
    /// update (e.g. `set_status` called right after an `append` that itself
    /// already flushed and drained `pending` at the batch/snapshot boundary).
    header_dirty: bool,
    /// When the store last took this run's events (construction counts: the
    /// accepted run is flushed right after it).
    last_flush: Instant,
}

/// Appends run events to the store in bounded write-behind batches, advancing
/// the header and driving the snapshot cadence. Shared `&`-only with worker
/// threads.
///
/// Interior mutability is a `parking_lot::Mutex` over the header state.
/// Lock discipline: the state lock is NEVER held across a redb store call —
/// every store write happens in [`Journaler::flush`], which takes the lock
/// itself only to drain the pending buffer, and no caller holds the lock
/// while invoking it.
pub struct Journaler<'a> {
    store: &'a Store,
    obs: Arc<dyn pii_core::Observer>,
    state: Mutex<JournalState>,
    bb: Arc<RwLock<Blackboard>>,
    /// Flush the pending buffer once it reaches this many events.
    batch: usize,
}

impl<'a> Journaler<'a> {
    /// A journaler for a fresh run whose header starts at seq 0, `Running`,
    /// stamped with the store's origin and `created_ms` (spec 188).
    #[must_use]
    pub fn fresh(
        store: &'a Store,
        obs: Arc<dyn pii_core::Observer>,
        run: RunId,
        graph_hash: String,
        bb: Arc<RwLock<Blackboard>>,
        created_ms: u64,
    ) -> Self {
        let mut header = RunHeader {
            run,
            graph: graph_hash,
            status: RunStatus::Running,
            seq: 0,
            dur_ms: 0,
            revision: String::new(),
            process_start_ms: 0,
            created_ms: 0,
        };
        store.stamp_origin(&mut header, created_ms);
        Self::with_batch(store, obs, header, bb, DEFAULT_BATCH)
    }

    /// A journaler that adopts a persisted header (resume continues the seq).
    #[must_use]
    pub fn adopt(
        store: &'a Store,
        obs: Arc<dyn pii_core::Observer>,
        header: RunHeader,
        bb: Arc<RwLock<Blackboard>>,
    ) -> Self {
        Self::with_batch(store, obs, header, bb, DEFAULT_BATCH)
    }

    /// A journaler with an explicit write-behind batch threshold (tests use a
    /// small batch to force flush-heavy behaviour; a huge batch to force
    /// buffering). Spec 11: the batch size is a constructor parameter, not a
    /// config knob.
    #[must_use]
    pub fn with_batch(
        store: &'a Store,
        obs: Arc<dyn pii_core::Observer>,
        header: RunHeader,
        bb: Arc<RwLock<Blackboard>>,
        batch: usize,
    ) -> Self {
        let last_seq = header.seq.saturating_sub(1);
        Self {
            store,
            obs,
            state: Mutex::new(JournalState {
                header,
                last_seq,
                pending: Vec::new(),
                header_dirty: false,
                last_flush: Instant::now(),
            }),
            bb,
            batch: batch.max(1),
        }
    }

    /// The store handle (for `put_resume_key` and content-addressed bodies).
    #[must_use]
    pub fn store(&self) -> &'a Store {
        self.store
    }

    /// The next sequence number to be written (accounts for buffered events).
    #[must_use]
    pub fn seq(&self) -> u64 {
        let st = self.state.lock();
        st.header.seq + st.pending.len() as u64
    }

    /// Sets the header status (call before appending the `Finished` event so
    /// the run is listed under its final status). Flushes any pending buffer
    /// FIRST so the status transition is durable before the event that
    /// carries it (spec 11 J2 flush barrier).
    pub fn set_status(&self, status: RunStatus) {
        {
            let mut st = self.state.lock();
            st.header.status = status;
            st.header_dirty = true;
        }
        if let Err(e) = self.flush() {
            self.report_flush_failure(&e);
        }
    }

    /// Surfaces a flush failure that the caller cannot propagate through the
    /// observer's persist-error channel (events stay buffered for a retry).
    pub fn report_flush_failure(&self, e: &PiiError) {
        let run = self.state.lock().header.run.clone();
        self.obs.event(&ObsEvent::PersistError {
            context: format!("journal:{run}"),
            message: e.to_string(),
        });
    }

    /// Records the total run duration on the header.
    pub fn set_dur_ms(&self, ms: u32) {
        let mut st = self.state.lock();
        st.header.dur_ms = ms;
        st.header_dirty = true;
    }

    /// Appends `event`: assigns the seq eagerly, notifies the observer
    /// IMMEDIATELY (live view — TUI / subscribe bus), buffers the event for
    /// the durable store journal, and flushes when the batch threshold is
    /// reached or a snapshot boundary is crossed. Returns the seq written.
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] on a redb or serialisation failure.
    pub fn append(&self, event: &RunEvent) -> Result<u64, PiiError> {
        let seq = {
            let mut st = self.state.lock();
            let seq = st.header.seq + st.pending.len() as u64;
            st.pending.push(event.clone());
            seq
        };
        // Observer fires on EVERY append (live view never lags).
        self.obs.event(&ObsEvent::Journal {
            seq,
            event: event.clone(),
        });
        if seq > 0 && seq.is_multiple_of(SNAPSHOT_EVERY) {
            // Snapshot boundary: flush (making the prefix durable through
            // this seq), then snapshot the blackboard anchored at this seq.
            self.flush()?;
            self.snapshot_at(seq)?;
        } else if self.pending_len() >= self.batch {
            self.flush()?;
        }
        Ok(seq)
    }

    /// The number of events currently buffered (write-behind pending).
    #[must_use]
    pub fn pending_len(&self) -> usize {
        self.state.lock().pending.len()
    }

    /// Flushes any pending events to the store in one write transaction.
    /// Returns the seq of the LAST event flushed (`last_seq` when nothing was
    /// pending). Used by `run.rs`'s `finish` path after appending the
    /// `Finished` event so a completed run's journal is fully durable, by the
    /// append path when the batch threshold / snapshot boundary hits, and by
    /// [`Journaler::snapshot_now`] which anchors the snapshot at the returned
    /// seq.
    ///
    /// The state lock is held across the store write: the journaler is the
    /// single writer for its run, and holding the lock serialises concurrent
    /// appends (they block in `append` until the flush commits) so header.seq
    /// and the store journal cannot diverge. Callers must not hold the lock
    /// when invoking `flush` — `append`/`set_status`/`finish` all drop it
    /// first.
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] on a redb or serialisation failure.
    pub fn flush(&self) -> Result<u64, PiiError> {
        let mut st = self.state.lock();
        if st.pending.is_empty() && !st.header_dirty {
            return Ok(st.last_seq);
        }
        // `pending` is cleared only AFTER `append_events` succeeds, so a
        // failed write leaves every event buffered for the next flush (D5).
        // The batch is bounded by `batch` (write-behind threshold) or the
        // pre-finish barrier — never the whole journal. It may be empty (a
        // header-only flush, e.g. `set_status` after a prior `append` already
        // flushed at a batch/snapshot boundary); `append_events` still
        // persists the header unconditionally.
        let mut header = st.header.clone();
        let refs: Vec<&RunEvent> = st.pending.iter().collect();
        self.store.append_events(&mut header, &refs)?;
        st.header = header;
        st.pending.clear();
        st.header_dirty = false;
        st.last_flush = Instant::now();
        st.last_seq = st.header.seq.saturating_sub(1);
        Ok(st.last_seq)
    }

    /// The run's journal as this writer sees it: the durable prefix plus the events still
    /// buffered, each with the seq it will land on. A read-only view — no write transaction,
    /// and no flush (spec 221's verdict reads this instead of forcing the store durable first;
    /// measured 2026-10-20: the forced flush was the third transaction of a coder-free run).
    ///
    /// # Errors
    /// Returns [`PiiError`] when the durable prefix cannot be read.
    pub fn events(&self) -> Result<Vec<(u64, RunEvent)>, PiiError> {
        let (run, base, pending) = {
            let st = self.state.lock();
            (st.header.run.clone(), st.header.seq, st.pending.clone())
        };
        let mut out = self.store.read_journal(&run)?;
        out.extend(
            pending
                .into_iter()
                .enumerate()
                .map(|(i, ev)| (base + i as u64, ev)),
        );
        Ok(out)
    }

    /// Time since the store last took this run's events.
    #[must_use]
    pub fn since_flush(&self) -> Duration {
        self.state.lock().last_flush.elapsed()
    }

    /// The in-flight barrier, scoped by time: flushes only when events are
    /// pending and the last flush is at least `window` old, so a run pays at
    /// most one transaction per window rather than one per node start.
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] on a redb or serialisation failure.
    pub fn flush_if_stale(&self, window: Duration) -> Result<(), PiiError> {
        {
            let st = self.state.lock();
            if st.pending.is_empty() || st.last_flush.elapsed() < window {
                return Ok(());
            }
        }
        self.flush().map(|_| ())
    }

    /// Snapshots the blackboard at the most recently flushed seq (used
    /// unconditionally on suspend). Flushes any pending buffer FIRST so the
    /// snapshot anchors a durable journal prefix (spec 11 J2).
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] on a redb or serialisation failure.
    pub fn snapshot_now(&self) -> Result<u64, PiiError> {
        let seq = self.flush()?;
        let run = self.state.lock().header.run.clone();
        Self::snapshot_locked(&self.bb, self.store, &run, seq)?;
        Ok(seq)
    }

    /// Snapshots the blackboard at an explicit seq (used when an append
    /// crossed a [`SNAPSHOT_EVERY`] boundary — the flush already made the
    /// prefix durable through `seq`, so the snapshot anchors there).
    fn snapshot_at(&self, seq: u64) -> Result<(), PiiError> {
        let run = self.state.lock().header.run.clone();
        Self::snapshot_locked(&self.bb, self.store, &run, seq)
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
