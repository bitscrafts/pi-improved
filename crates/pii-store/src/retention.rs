//! Spec 376: retention. A sweep drops finished runs and node-call events older than the policy, removes at
//! most `max_rows` rows per call, and never touches a live run (`Running` or `Suspended`).
//!
//! A journal event carries no wall-clock time, so an event's age is its run's `created_ms`.
//! A sweep reports its count through the store's observer as one `store.retention.swept` event.

use pii_abi::{PiiError, RunEvent, RunHeader, RunStatus};
use pii_observe::{Event, Level};
use redb::{ReadableTable, TableDefinition, WriteTransaction};

use crate::tables::{RUN, RUN_BY_STATUS, RUN_JOURNAL, RUN_SNAPSHOT};
use crate::{Store, from_bytes, status_name};

/// Event kind a sweep that removed rows emits (spec 376 R5).
pub const RETENTION_SWEPT: &str = "store.retention.swept";

const DAY_MS: u64 = 86_400_000;

/// What a sweep expires. Defaults: run journals after 30 days, node-call events after 7, 1 000 rows per call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionPolicy {
    /// Age past which a finished run's header, journal and snapshots go.
    pub run_age_ms: u64,
    /// Age past which a finished run's node-call events go.
    pub node_event_age_ms: u64,
    /// The most rows one sweep removes.
    pub max_rows: usize,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            run_age_ms: 30 * DAY_MS,
            node_event_age_ms: 7 * DAY_MS,
            max_rows: 1_000,
        }
    }
}

impl Store {
    /// Runs one retention sweep at wall-clock `now_ms` inside one write transaction, and returns how many rows
    /// it removed. Node-call events go first, then whole runs; the cap applies to both.
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] on a redb or decode failure. Nothing is committed then.
    pub fn sweep_retention(
        &self,
        now_ms: u64,
        policy: &RetentionPolicy,
    ) -> Result<usize, PiiError> {
        let write = self.begin_write_counted()?;
        let youngest_expired = policy.run_age_ms.min(policy.node_event_age_ms);
        // A row we cannot date is a row we cannot reason about, so it is kept: `created_ms == 0`
        // is a store written before the clock was recorded (and every test fixture), and deleting
        // on a missing timestamp would quietly destroy exactly the runs an operator is looking for.
        let candidates: Vec<RunHeader> = finished_runs(&write)?
            .into_iter()
            .filter(|h| h.created_ms > 0)
            .filter(|h| h.created_ms.saturating_add(youngest_expired) <= now_ms)
            .collect();

        let mut budget = policy.max_rows;
        let mut removed = 0;
        for h in &candidates {
            if h.created_ms.saturating_add(policy.node_event_age_ms) <= now_ms {
                let n = drop_node_events(&write, h.run.as_str(), budget)?;
                budget -= n;
                removed += n;
            }
        }
        for h in &candidates {
            if budget == 0 {
                break;
            }
            if h.created_ms.saturating_add(policy.run_age_ms) <= now_ms {
                let n = drop_run(&write, h, budget)?;
                budget -= n;
                removed += n;
            }
        }

        write
            .commit()
            .map_err(|e| PiiError::store_op("commit", "retention", e))?;
        if let Some(observer) = self.observer.get().filter(|_| removed > 0) {
            observer.event(
                &Event::new("store", Level::Info, RETENTION_SWEPT).field("removed", removed as u64),
            );
        }
        Ok(removed)
    }
}

/// Every run whose status is not live, read in this transaction.
fn finished_runs(write: &WriteTransaction) -> Result<Vec<RunHeader>, PiiError> {
    let table = write
        .open_table(RUN)
        .map_err(|e| PiiError::store_op("open", "run", e))?;
    let mut out = Vec::new();
    for item in table
        .iter()
        .map_err(|e| PiiError::store_op("range", "run", e))?
    {
        let (_, bytes) = item.map_err(|e| PiiError::store_op("read", "run", e))?;
        let header: RunHeader = from_bytes(bytes.value())?;
        if !matches!(header.status, RunStatus::Running | RunStatus::Suspended) {
            out.push(header);
        }
    }
    Ok(out)
}

/// Removes up to `budget` node-call events of `run`; returns how many.
fn drop_node_events(write: &WriteTransaction, run: &str, budget: usize) -> Result<usize, PiiError> {
    let mut journal = write
        .open_table(RUN_JOURNAL)
        .map_err(|e| PiiError::store_op("open", "journal", e))?;
    let mut doomed = Vec::new();
    for item in journal
        .range((run, 0)..=(run, u64::MAX))
        .map_err(|e| PiiError::store_op("range", "journal", e))?
    {
        if doomed.len() == budget {
            break;
        }
        let (key, bytes) = item.map_err(|e| PiiError::store_op("read", "journal", e))?;
        let event: RunEvent = from_bytes(bytes.value())?;
        if matches!(
            event,
            RunEvent::NodeStarted { .. }
                | RunEvent::NodeFinished { .. }
                | RunEvent::NodeFailed { .. }
                | RunEvent::NodeProgress { .. }
        ) {
            doomed.push(key.value().1);
        }
    }
    for seq in &doomed {
        journal
            .remove((run, *seq))
            .map_err(|e| PiiError::store_op("remove", "event", e))?;
    }
    Ok(doomed.len())
}

/// Removes a finished run's header, status entry, journal and snapshots, as many as `budget` allows. The
/// header goes last, once nothing of the run is left, so a capped sweep never orphans rows. Returns the rows
/// removed; the status-index entry goes with the header and is not counted.
fn drop_run(
    write: &WriteTransaction,
    header: &RunHeader,
    budget: usize,
) -> Result<usize, PiiError> {
    let run = header.run.as_str();
    let (journal, journal_done) = drop_seq_rows(write, RUN_JOURNAL, run, budget)?;
    if !journal_done {
        return Ok(journal);
    }
    let (snapshots, snapshots_done) = drop_seq_rows(write, RUN_SNAPSHOT, run, budget - journal)?;
    if !snapshots_done {
        return Ok(journal + snapshots);
    }
    write
        .open_table(RUN)
        .map_err(|e| PiiError::store_op("open", "run", e))?
        .remove(run)
        .map_err(|e| PiiError::store_op("remove", "run header", e))?;
    write
        .open_table(RUN_BY_STATUS)
        .map_err(|e| PiiError::store_op("open", "status", e))?
        .remove((status_name(header.status), run))
        .map_err(|e| PiiError::store_op("remove", "status", e))?;
    Ok(journal + snapshots + 1)
}

/// Removes up to `budget` `(run, seq)` rows of `def`. Returns the count and whether the run's rows are all gone.
fn drop_seq_rows(
    write: &WriteTransaction,
    def: TableDefinition<'_, (&str, u64), &[u8]>,
    run: &str,
    budget: usize,
) -> Result<(usize, bool), PiiError> {
    let mut table = write
        .open_table(def)
        .map_err(|e| PiiError::store_op("open", "run rows", e))?;
    let seqs: Vec<u64> = table
        .range((run, 0)..=(run, u64::MAX))
        .map_err(|e| PiiError::store_op("range", "run rows", e))?
        .take(budget)
        .map(|item| {
            item.map(|(key, _)| key.value().1)
                .map_err(|e| PiiError::store_op("read", "run rows", e))
        })
        .collect::<Result<_, _>>()?;
    for seq in &seqs {
        table
            .remove((run, *seq))
            .map_err(|e| PiiError::store_op("remove", "run rows", e))?;
    }
    Ok((seqs.len(), seqs.len() < budget))
}

/// One sweep at daemon boot (spec 376 R4).
///
/// A failure is reported to the caller's sink and is not a refusal: the daemon starts with a store
/// that is merely no smaller — refusing to boot over retention would trade a big store for none.
pub fn sweep_at_boot(store: &Store, now_ms: u64, on_error: impl FnOnce(&PiiError)) {
    if let Err(e) = store.sweep_retention(now_ms, &RetentionPolicy::default()) {
        on_error(&e);
    }
}
