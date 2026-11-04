//! Spec 370: storage under the health record. What the record means is owned by `pii-server`; this module gives it two
//! tables, a rebuild walk over every run, and an observer that runs inside the append transaction, so a record
//! updated by the observer commits with the events it counts or not at all.

use pii_abi::{PiiError, RunEvent, RunHeader};
use redb::{ReadableDatabase, ReadableTable, TableDefinition};

use crate::tables::{RUN, RUN_JOURNAL, SYS_STATUS, SYS_STATUS_RUN};
use crate::{Store, WriteTransaction};

/// Called inside the append transaction with the run's header after the append (its status and seq already moved),
/// and the events just appended. An error aborts the append.
pub type AppendObserver =
    fn(&Store, &WriteTransaction, &RunHeader, &[&RunEvent]) -> Result<(), PiiError>;

/// The two tables the health record lives in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SysTable {
    /// The record itself, under its own key.
    Totals,
    /// One row per run, keyed by run id.
    Runs,
}

fn def(table: SysTable) -> TableDefinition<'static, &'static str, &'static [u8]> {
    match table {
        SysTable::Totals => SYS_STATUS,
        SysTable::Runs => SYS_STATUS_RUN,
    }
}

/// Reads `key` from `table` inside `write`.
///
/// # Errors
/// Returns [`PiiError::Internal`] on a redb failure.
pub fn sys_get(
    write: &WriteTransaction,
    table: SysTable,
    key: &str,
) -> Result<Option<Vec<u8>>, PiiError> {
    let tbl = write
        .open_table(def(table))
        .map_err(|e| PiiError::store_op("open", "sys_status", e))?;
    let value = tbl
        .get(key)
        .map_err(|e| PiiError::store_op("read", "sys_status", e))?
        .map(|v| v.value().to_vec());
    Ok(value)
}

/// Writes `key` in `table` inside `write`.
///
/// # Errors
/// Returns [`PiiError::Internal`] on a redb failure.
pub fn sys_put(
    write: &WriteTransaction,
    table: SysTable,
    key: &str,
    bytes: &[u8],
) -> Result<(), PiiError> {
    let mut tbl = write
        .open_table(def(table))
        .map_err(|e| PiiError::store_op("open", "sys_status", e))?;
    tbl.insert(key, bytes)
        .map_err(|e| PiiError::store_op("put", "sys_status", e))?;
    Ok(())
}

/// Empties both record tables inside `write`. They are re-created by the next write.
///
/// # Errors
/// Returns [`PiiError::Internal`] on a redb failure.
pub fn sys_clear(write: &WriteTransaction) -> Result<(), PiiError> {
    write
        .delete_table(SYS_STATUS)
        .map_err(|e| PiiError::store_op("delete", "sys_status", e))?;
    write
        .delete_table(SYS_STATUS_RUN)
        .map_err(|e| PiiError::store_op("delete", "sys_status_run", e))?;
    Ok(())
}

/// How many runs the store holds, counting no more than `limit`, read inside `write`.
///
/// # Errors
/// Returns [`PiiError::Internal`] on a redb failure.
pub fn runs_up_to(write: &WriteTransaction, limit: usize) -> Result<usize, PiiError> {
    let runs = write
        .open_table(RUN)
        .map_err(|e| PiiError::store_op("open", "run", e))?;
    let mut seen = 0;
    for item in runs
        .iter()
        .map_err(|e| PiiError::store_op("iter", "run", e))?
        .take(limit)
    {
        item.map_err(|e| PiiError::store_op("read", "run", e))?;
        seen += 1;
    }
    Ok(seen)
}

/// Visits every run in `write`, oldest key first, with its header and its whole journal in seq order.
///
/// # Errors
/// Returns [`PiiError::Internal`] on a redb or decode failure, or the first error `f` returns.
pub fn for_each_run_journal(
    write: &WriteTransaction,
    mut f: impl FnMut(&RunHeader, &[&RunEvent]) -> Result<(), PiiError>,
) -> Result<(), PiiError> {
    let runs = write
        .open_table(RUN)
        .map_err(|e| PiiError::store_op("open", "run", e))?;
    let journal = write
        .open_table(RUN_JOURNAL)
        .map_err(|e| PiiError::store_op("open", "run_journal", e))?;
    for item in runs
        .iter()
        .map_err(|e| PiiError::store_op("iter", "run", e))?
    {
        let (_, value) = item.map_err(|e| PiiError::store_op("read", "run", e))?;
        let header: RunHeader = crate::from_bytes(value.value())?;
        let run = header.run.as_str();
        let mut owned = Vec::new();
        for item in journal
            .range((run, 0u64)..=(run, u64::MAX))
            .map_err(|e| PiiError::store_op("range", "run_journal", e))?
        {
            let (_, value) = item.map_err(|e| PiiError::store_op("read", "run_journal", e))?;
            owned.push(crate::from_bytes::<RunEvent>(value.value())?);
        }
        let refs: Vec<&RunEvent> = owned.iter().collect();
        f(&header, &refs)?;
    }
    Ok(())
}

use std::sync::atomic::Ordering;

impl Store {
    /// Installs the observer that every append runs inside its transaction. Replaces any earlier one.
    pub fn set_append_observer(&self, observer: AppendObserver) {
        *self.append_observer.lock() = Some(observer);
    }

    /// The installed append observer, if any.
    pub(crate) fn append_observer(&self) -> Option<AppendObserver> {
        *self.append_observer.lock()
    }

    /// Reads `key` from `table` outside any write.
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] on a redb failure.
    pub fn sys_read(&self, table: SysTable, key: &str) -> Result<Option<Vec<u8>>, PiiError> {
        let read = self
            .db
            .begin_read()
            .map_err(|e| PiiError::store_op("begin_read", "db", e))?;
        let tbl = read
            .open_table(def(table))
            .map_err(|e| PiiError::store_op("open", "sys_status", e))?;
        let value = tbl
            .get(key)
            .map_err(|e| PiiError::store_op("read", "sys_status", e))?
            .map(|v| v.value().to_vec());
        Ok(value)
    }

    /// Runs `f` in one counted write transaction and commits it when `f` succeeds.
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] on a redb failure, or the error `f` returns.
    pub fn with_write<T>(
        &self,
        f: impl FnOnce(&WriteTransaction) -> Result<T, PiiError>,
    ) -> Result<T, PiiError> {
        let write = self.begin_write_counted()?;
        let out = f(&write)?;
        write
            .commit()
            .map_err(|e| PiiError::store_op("commit", "sys_status", e))?;
        Ok(out)
    }
}

impl Store {
    /// Counts the write transactions the store performs, so a test can prove a path costs no barrier.
    ///
    /// Lives here rather than in `lib.rs` because this module owns the counters the number feeds, and
    /// `lib.rs` is at its line budget.
    pub(crate) fn begin_write_counted(&self) -> Result<WriteTransaction, PiiError> {
        self.write_txn_count.fetch_add(1, Ordering::Relaxed);
        self.db
            .begin_write()
            .map_err(|e| PiiError::store_op("begin_write", "db", e))
    }
}

impl Store {
    /// Write transactions this store has opened, for the tests that prove a path costs no barrier.
    #[must_use]
    pub fn write_txn_count(&self) -> u64 {
        self.write_txn_count.load(Ordering::Relaxed)
    }
}
