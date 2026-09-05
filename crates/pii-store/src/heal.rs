//! Heal-attempt ledger (S0.1): `heal_attempt: signature_hash -> encoded
//! HealAttempt`. Same structure as `ApprovalStore` (open/put/get per
//! method; no held transaction).

use pii_abi::PiiError;
use redb::{Database, ReadableDatabase, ReadableTable};
use serde::{Deserialize, Serialize};

use crate::codec::{from_bytes, to_bytes};
use crate::tables::HEAL_ATTEMPT;

/// One recorded run of a failing gate (S0.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealAttemptRun {
    /// The run id that executed the check.
    pub run_id: String,
    /// Outcome: `"detected"` or `"flake"`.
    pub outcome: String,
    /// Epoch millis.
    pub ts_ms: u64,
    /// Failure detail (first bytes of the gate output).
    pub detail: String,
}

/// One reproducible failure signature (S0.1). Deterministic `signature_hash`
/// so re-detecting the SAME failure updates this row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealAttempt {
    /// `blake3(gate, first-N bytes of failure output)` hex.
    pub signature_hash: String,
    /// The gate (`"quality-gate"`, `"lint-files"`, …).
    pub gate: String,
    /// First seen epoch millis.
    pub first_seen_ms: u64,
    /// Last seen epoch millis.
    pub last_seen_ms: u64,
    /// Every recorded run of this signature.
    pub attempts: Vec<HealAttemptRun>,
}

/// The heal-attempt ledger (S0.1).
pub struct HealStore {
    db: Database,
}

impl HealStore {
    /// Opens (creating if needed) the heal store on `db`.
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb failure.
    pub fn open(path: &std::path::Path) -> Result<Self, PiiError> {
        let db = Database::create(path)
            .map_err(|e| PiiError::Internal(format!("heal: open {}: {e}", path.display())))?;
        {
            let write = db
                .begin_write()
                .map_err(|e| PiiError::Internal(format!("heal: begin_write: {e}")))?;
            write
                .open_table(HEAL_ATTEMPT)
                .map_err(|e| PiiError::Internal(format!("heal: open table: {e}")))?;
            write
                .commit()
                .map_err(|e| PiiError::Internal(format!("heal: commit tables: {e}")))?;
        }
        Ok(Self { db })
    }

    /// Inserts a new attempt row or updates an existing signature (S0.1).
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb or encode failure.
    pub fn put_attempt(&self, a: &HealAttempt) -> Result<(), PiiError> {
        let bytes = to_bytes(a)?;
        let write = self
            .db
            .begin_write()
            .map_err(|e| PiiError::Internal(format!("heal: begin_write: {e}")))?;
        {
            let mut t = write
                .open_table(HEAL_ATTEMPT)
                .map_err(|e| PiiError::Internal(format!("heal: open table: {e}")))?;
            t.insert(a.signature_hash.as_str(), bytes.as_slice())
                .map_err(|e| PiiError::Internal(format!("heal: insert: {e}")))?;
        }
        write
            .commit()
            .map_err(|e| PiiError::Internal(format!("heal: commit put: {e}")))
    }

    /// Reads one attempt by signature hash (`None` when unknown).
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb or decode failure.
    pub fn get_attempt(&self, sig: &str) -> Result<Option<HealAttempt>, PiiError> {
        let read = self
            .db
            .begin_read()
            .map_err(|e| PiiError::Internal(format!("heal: begin_read: {e}")))?;
        let t = read
            .open_table(HEAL_ATTEMPT)
            .map_err(|e| PiiError::Internal(format!("heal: open table: {e}")))?;
        let Some(v) = t
            .get(sig)
            .map_err(|e| PiiError::Internal(format!("heal: get: {e}")))?
        else {
            return Ok(None);
        };
        Ok(Some(from_bytes(v.value())?))
    }

    /// Every recorded attempt, in signature order.
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb or decode failure.
    pub fn list_attempts(&self) -> Result<Vec<HealAttempt>, PiiError> {
        let read = self
            .db
            .begin_read()
            .map_err(|e| PiiError::Internal(format!("heal: begin_read: {e}")))?;
        let t = read
            .open_table(HEAL_ATTEMPT)
            .map_err(|e| PiiError::Internal(format!("heal: open table: {e}")))?;
        let mut out = Vec::new();
        for row in t
            .iter()
            .map_err(|e| PiiError::Internal(format!("heal: iter: {e}")))?
        {
            let row = row.map_err(|e| PiiError::Internal(format!("heal: row: {e}")))?;
            out.push(from_bytes(row.1.value())?);
        }
        Ok(out)
    }

    /// Adds one run to an existing signature, or creates the signature.
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb or encode failure.
    pub fn record_run(
        &self,
        sig: &str,
        gate: &str,
        run: HealAttemptRun,
    ) -> Result<HealAttempt, PiiError> {
        let mut existing = self.get_attempt(sig)?.unwrap_or(HealAttempt {
            signature_hash: sig.to_owned(),
            gate: gate.to_owned(),
            first_seen_ms: run.ts_ms,
            last_seen_ms: run.ts_ms,
            attempts: Vec::new(),
        });
        existing.last_seen_ms = run.ts_ms;
        existing.attempts.push(run);
        self.put_attempt(&existing)?;
        Ok(existing)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use tempfile::Builder;

    use super::{HealAttemptRun, HealStore};

    #[test]
    fn put_get_list_roundtrip() {
        let dir = Builder::new()
            .prefix("heal-store-")
            .tempdir_in("/projects/pi-improved/_tmp")
            .unwrap();
        let store = HealStore::open(&dir.path().join("heal.redb")).unwrap();
        let run = HealAttemptRun {
            run_id: String::from("r1"),
            outcome: String::from("detected"),
            ts_ms: 100,
            detail: String::from("quality-gate: FAIL"),
        };
        let a = store.record_run("abc123", "quality-gate", run).unwrap();
        assert_eq!(a.attempts.len(), 1);
        assert_eq!(
            store.get_attempt("abc123").unwrap().unwrap().gate,
            "quality-gate"
        );
        // Same signature updates, does not duplicate.
        let run2 = HealAttemptRun {
            run_id: String::from("r2"),
            outcome: String::from("detected"),
            ts_ms: 200,
            detail: String::from("quality-gate: FAIL"),
        };
        let b = store.record_run("abc123", "quality-gate", run2).unwrap();
        assert_eq!(b.attempts.len(), 2);
        assert_eq!(store.list_attempts().unwrap().len(), 1);
    }
}
