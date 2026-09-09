//! Heal-attempt ledger (S0.1): `heal_attempt: signature_hash -> encoded
//! HealAttempt`. Same structure as `ApprovalStore` (open/put/get per
//! method; no held transaction).

use pii_abi::PiiError;
use redb::{Database, ReadableDatabase, ReadableTable};
use serde::{Deserialize, Serialize};

use crate::codec::{from_bytes, to_bytes};
use crate::tables::HEAL_ATTEMPT;

/// State of a heal attempt (spec 26 R3): typed enum replacing the stringly
/// `outcome`. Lowercase serde keeps old rows readable; `Default` is `Detected`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum HealState {
    /// The failure was first detected (gate red).
    #[default]
    Detected,
    /// A re-check did not reproduce the failure.
    Flake,
    /// A patch resolved the failing gate.
    Fixed,
    /// The fixed build was applied/validated.
    Applied,
    /// Escalated to a human (never auto-promoted).
    Escalated,
}

/// The human decision attached to a promotion (spec 26 R3), bound to the id
/// the `gate` node mints — so the `Approval.id` and the ledger's decision join.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalDecision {
    /// `true` = allow, `false` = deny.
    pub allow: bool,
    /// The `approval_id` the gate node minted (`blake3(run,node,cycle)[..16]`).
    pub approval_id: String,
    /// Who resolved it (approver identity / note).
    pub by: String,
    /// Epoch millis.
    pub ts_ms: u64,
}

/// One recorded run of a failing gate (S0.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealAttemptRun {
    /// The run id that executed the check.
    pub run_id: String,
    /// Outcome (`"detected"` or `"flake"`) — typed since spec 26.
    pub outcome: HealState,
    /// Epoch millis.
    pub ts_ms: u64,
    /// Failure detail (first bytes of the gate output).
    pub detail: String,
}

/// One reproducible failure signature (S0.1). Deterministic `signature_hash`
/// so re-detecting the SAME failure updates this row.
///
/// Spec 26 R3 (promotion prerequisites) evolved this from the Slice-0 shape:
/// a typed `HealState`, content-addressed patch, worktree/branch provenance,
/// before/after gate evidence, the human decision bound to the approval id,
/// cost, and a bounded attempt ring. Every new field is `#[serde(default)]`
/// so old rows still deserialize (R5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealAttempt {
    /// `blake3(gate, first-N bytes of failure output)` hex.
    pub signature_hash: String,
    /// The gate (`"quality-gate"`, `"lint-files"`, …).
    pub gate: String,
    /// Current state of the attempt.
    #[serde(default)]
    pub state: HealState,
    /// First seen epoch millis.
    pub first_seen_ms: u64,
    /// Last seen epoch millis.
    pub last_seen_ms: u64,
    /// Every recorded run of this signature (bounded ring, default cap 20).
    pub attempts: Vec<HealAttemptRun>,
    /// Total attempts ever (never lost to ring eviction).
    #[serde(default)]
    pub attempt_count: u32,
    /// Hash of the patch content, content-addressed via the CAS (never inline).
    #[serde(default)]
    pub patch_hash: Option<String>,
    /// Disposable worktree the patch was made in.
    #[serde(default)]
    pub worktree: Option<String>,
    /// Branch the patch commits onto.
    #[serde(default)]
    pub branch: Option<String>,
    /// Base commit the patch was authored against.
    #[serde(default)]
    pub base_commit: Option<String>,
    /// The gate result BEFORE the patch (`"fail"` / `"pass"` / gate exit code).
    #[serde(default)]
    pub gate_before: Option<String>,
    /// The gate result AFTER the patch.
    #[serde(default)]
    pub gate_after: Option<String>,
    /// The human decision, bound to the approval id the gate node minted.
    #[serde(default)]
    pub decision: Option<ApprovalDecision>,
    /// Model used for the patch (if a model authored it).
    #[serde(default)]
    pub model: Option<String>,
    /// Token / cost accounting.
    #[serde(default)]
    pub tokens_in: u64,
    /// Token / cost accounting.
    #[serde(default)]
    pub tokens_out: u64,
    /// Micro-dollar spend.
    #[serde(default)]
    pub micro_usd: u64,
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
        // RSS-gate fix (2026-09-05): redb defaults to a 1 GiB cache; this
        // store is small (approvals/channels/heal/memory/timers), so 4 MiB
        // is ample and avoids each auxiliary store compounding the
        // daemon's total cache footprint under soak (see pii-store/src/
        // lib.rs's Store::open for the full rationale).
        let db = Database::builder()
            .set_cache_size(4 * 1024 * 1024)
            .create(path)
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
    /// Keeps `attempt_count` monotonic and bounds `attempts` to a ring of
    /// `ATTEMPT_RING_CAP` (oldest evicted, count never lost) per spec 26 R3.
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb or encode failure.
    pub fn record_run(
        &self,
        sig: &str,
        gate: &str,
        run: HealAttemptRun,
    ) -> Result<HealAttempt, PiiError> {
        let mut existing = self.get_attempt(sig)?.unwrap_or_else(|| HealAttempt {
            signature_hash: sig.to_owned(),
            gate: gate.to_owned(),
            state: HealState::Detected,
            first_seen_ms: run.ts_ms,
            last_seen_ms: run.ts_ms,
            attempts: Vec::new(),
            attempt_count: 0,
            patch_hash: None,
            worktree: None,
            branch: None,
            base_commit: None,
            gate_before: None,
            gate_after: None,
            decision: None,
            model: None,
            tokens_in: 0,
            tokens_out: 0,
            micro_usd: 0,
        });
        existing.last_seen_ms = run.ts_ms;
        existing.attempt_count = existing.attempt_count.saturating_add(1);
        existing.attempts.push(run);
        while existing.attempts.len() > ATTEMPT_RING_CAP {
            existing.attempts.remove(0);
        }
        self.put_attempt(&existing)?;
        Ok(existing)
    }
}

/// Max attempts kept in the ring (older runs evicted; `attempt_count` keeps
/// the true total). Spec 26 R3 — a human approving a promotion sees the
/// recent attempts plus an honest total.
pub const ATTEMPT_RING_CAP: usize = 20;

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use tempfile::Builder;

    use super::{ATTEMPT_RING_CAP, HealAttemptRun, HealState, HealStore};

    #[test]
    fn put_get_list_roundtrip() {
        let dir = Builder::new()
            .prefix("heal-store-")
            .tempdir_in("/projects/pi-improved/_tmp")
            .unwrap();
        let store = HealStore::open(&dir.path().join("heal.redb")).unwrap();
        let run = HealAttemptRun {
            run_id: String::from("r1"),
            outcome: HealState::Detected,
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
            outcome: HealState::Flake,
            ts_ms: 200,
            detail: String::from("quality-gate: FAIL"),
        };
        let b = store.record_run("abc123", "quality-gate", run2).unwrap();
        assert_eq!(b.attempts.len(), 2);
        assert_eq!(store.list_attempts().unwrap().len(), 1);
    }

    /// R3 (spec 26): the attempt ring caps at `ATTEMPT_RING_CAP` and
    /// `attempt_count` keeps the true total (never lost to eviction).
    #[test]
    fn attempt_ring_caps_and_counts() {
        let dir = Builder::new()
            .prefix("heal-ring-")
            .tempdir_in("/projects/pi-improved/_tmp")
            .unwrap();
        let store = HealStore::open(&dir.path().join("heal.redb")).unwrap();
        for i in 0..25u64 {
            store
                .record_run(
                    "ring-sig",
                    "quality-gate",
                    HealAttemptRun {
                        run_id: format!("r{i}"),
                        outcome: HealState::Detected,
                        ts_ms: i,
                        detail: String::new(),
                    },
                )
                .unwrap();
        }
        let a = store.get_attempt("ring-sig").unwrap().unwrap();
        assert_eq!(
            a.attempts.len(),
            ATTEMPT_RING_CAP,
            "attempts must be a bounded ring of {ATTEMPT_RING_CAP}"
        );
        assert_eq!(
            a.attempt_count, 25,
            "attempt_count must keep the true total"
        );
        // The oldest attempts were evicted from the ring.
        assert_eq!(a.attempts.first().map(|x| x.run_id.as_str()), Some("r5"));
        assert_eq!(a.attempts.last().map(|x| x.run_id.as_str()), Some("r24"));
    }

    /// R5 (spec 26): a legacy row written with `outcome: "detected"` (String)
    /// and no evolved fields still deserializes (serde default + lowercase enum).
    #[test]
    fn legacy_heal_row_deserializes() {
        use super::HealAttempt;
        // Exactly the pre-spec-26 serialized shape.
        let json = r#"{
            "signature_hash": "legacy",
            "gate": "quality-gate",
            "first_seen_ms": 1,
            "last_seen_ms": 2,
            "attempts": [{"run_id": "r1", "outcome": "detected", "ts_ms": 1, "detail": "FAIL"}]
        }"#;
        let row: HealAttempt = serde_json::from_str(json).expect("legacy row must deserialize");
        assert_eq!(row.state, HealState::Detected);
        assert_eq!(
            row.attempts.first().map(|x| x.outcome),
            Some(HealState::Detected)
        );
        assert_eq!(
            row.attempt_count, 0,
            "legacy row has no attempt_count (defaulted)"
        );
        assert_eq!(row.patch_hash, None);
        assert_eq!(row.decision, None);
    }
}
