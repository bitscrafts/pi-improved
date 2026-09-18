//! Timer wheel (F5.5): `timer_wheel: (fire_at_ms, run_id) -> resume_key`.
//! Firing is ONE write transaction per due batch; no transaction is ever
//! held across a call boundary.

use std::path::Path;

use pii_abi::PiiError;
use redb::{Database, ReadableDatabase, ReadableTable};

use crate::tables::TIMER_WHEEL;

/// The durable timer wheel. Cross-process safety comes from redb's write
/// lock: two daemons can never fire the same timer twice.
pub struct TimerWheel {
    db: Database,
}

/// One timer due for firing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueTimer {
    /// Fire-at epoch millis.
    pub fire_at_ms: u64,
    /// The run to resume.
    pub run: String,
    /// The resume key.
    pub key: String,
}

impl TimerWheel {
    /// Opens (creating if needed) the timer wheel on `db`.
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb failure.
    pub fn open(path: &Path) -> Result<Self, PiiError> {
        // RSS-gate fix (2026-09-05): redb defaults to a 1 GiB cache; this
        // store is small (approvals/channels/heal/memory/timers), so 4 MiB
        // is ample and avoids each auxiliary store compounding the
        // daemon's total cache footprint under soak (see pii-store/src/
        // lib.rs's Store::open for the full rationale).
        let db = Database::builder()
            .set_cache_size(4 * 1024 * 1024)
            .create(path)
            .map_err(|e| PiiError::Internal(format!("timer: open {}: {e}", path.display())))?;
        {
            let write = db
                .begin_write()
                .map_err(|e| PiiError::Internal(format!("timer: begin_write: {e}")))?;
            write
                .open_table(TIMER_WHEEL)
                .map_err(|e| PiiError::Internal(format!("timer: open table: {e}")))?;
            write
                .commit()
                .map_err(|e| PiiError::Internal(format!("timer: commit tables: {e}")))?;
        }
        Ok(Self { db })
    }

    /// Registers a timer (idempotent: re-put replaces).
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb failure.
    pub fn put_timer(&self, fire_at_ms: u64, run: &str, key: &str) -> Result<(), PiiError> {
        let write = self
            .db
            .begin_write()
            .map_err(|e| PiiError::Internal(format!("timer: begin_write: {e}")))?;
        {
            let mut t = write
                .open_table(TIMER_WHEEL)
                .map_err(|e| PiiError::Internal(format!("timer: open table: {e}")))?;
            t.insert((fire_at_ms, run), key)
                .map_err(|e| PiiError::Internal(format!("timer: insert: {e}")))?;
        }
        write
            .commit()
            .map_err(|e| PiiError::Internal(format!("timer: commit put: {e}")))
    }

    /// The earliest `fire_at_ms` still pending (`None` when empty).
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb failure.
    pub fn next_timer(&self) -> Result<Option<u64>, PiiError> {
        let read = self
            .db
            .begin_read()
            .map_err(|e| PiiError::Internal(format!("timer: begin_read: {e}")))?;
        let t = read
            .open_table(TIMER_WHEEL)
            .map_err(|e| PiiError::Internal(format!("timer: open table: {e}")))?;
        let first = t
            .first()
            .map_err(|e| PiiError::Internal(format!("timer: first: {e}")))?;
        Ok(first.map(|(k, _v)| k.value().0))
    }

    /// Takes every timer with `fire_at_ms <= now_ms` in ONE write txn (F5.5):
    /// readers cannot see a partial firing and a second daemon sees none.
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb failure.
    pub fn take_due(&self, now_ms: u64) -> Result<Vec<DueTimer>, PiiError> {
        let write = self
            .db
            .begin_write()
            .map_err(|e| PiiError::Internal(format!("timer: begin_write: {e}")))?;
        let mut due = Vec::new();
        {
            let mut t = write
                .open_table(TIMER_WHEEL)
                .map_err(|e| PiiError::Internal(format!("timer: open table: {e}")))?;
            let range = t
                .range(..(now_ms, "\u{10FFFF}"))
                .map_err(|e| PiiError::Internal(format!("timer: range: {e}")))?;
            for row in range {
                let Ok(row) = row else { continue };
                let (fire_at, run) = row.0.value();
                due.push(DueTimer {
                    fire_at_ms: fire_at,
                    run: String::from(run),
                    key: String::from(row.1.value()),
                });
            }
            for d in &due {
                t.remove((d.fire_at_ms, d.run.as_str()))
                    .map_err(|e| PiiError::Internal(format!("timer: remove: {e}")))?;
            }
        }
        write
            .commit()
            .map_err(|e| PiiError::Internal(format!("timer: commit take: {e}")))?;
        Ok(due)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::TimerWheel;

    fn scratch() -> std::path::PathBuf {
        let dir = std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/_tmp/phase3/timertest"
        ));
        let _ = std::fs::create_dir_all(dir);
        dir.join(format!("tw-{}.redb", std::process::id()))
    }

    #[test]
    fn timer_wheel_take_due_is_atomic() {
        let path = scratch();
        let w = TimerWheel::open(&path).unwrap();
        w.put_timer(100, "r1", "k1").unwrap();
        w.put_timer(200, "r2", "k2").unwrap();
        assert_eq!(w.next_timer().unwrap(), Some(100));
        let due = w.take_due(150).unwrap();
        assert_eq!(due.len(), 1, "only the 100 ms timer: {due:?}");
        let first = due.first().expect("one due timer");
        assert_eq!(first.run, "r1");
        // Taken entries are gone (no re-fire).
        assert_eq!(w.take_due(150).unwrap().len(), 0);
        let due2 = w.take_due(250).unwrap();
        assert_eq!(due2.len(), 1);
        let _ = std::fs::remove_file(&path);
    }
}
