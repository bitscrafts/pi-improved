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
    /// Product-specific fields the generic store need not know about
    /// (spec 46, F-14): a free-form bag for feature-owned data. Empty by
    /// default; old rows with now-removed fields still deserialize since
    /// `#[serde(default)]` maps any unknown-here JSON keys to nothing and
    /// this field simply starts empty.
    #[serde(default)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// The heal-attempt ledger (S0.1).
pub struct HealStore {
    db: Database,
}

impl HealStore {
    /// Opens (creating if needed) the heal store on `db`.
    ///
    /// # Errors
    /// [`PiiError::StoreOp`] on a redb failure.
    pub fn open(path: &std::path::Path) -> Result<Self, PiiError> {
        // RSS-gate fix (2026-09-05): redb defaults to a 1 GiB cache; this
        // store is small (approvals/channels/heal/memory/timers), so 4 MiB
        // is ample and avoids each auxiliary store compounding the
        // daemon's total cache footprint under soak (see pii-store/src/
        // lib.rs's Store::open for the full rationale).
        let db = Database::builder()
            .set_cache_size(4 * 1024 * 1024)
            .create(path)
            .map_err(|e| PiiError::StoreOp {
                op: String::from("open"),
                resource: String::from("heal_db"),
                detail: format!("{}: {e}", path.display()),
            })?;
        {
            let write = db.begin_write().map_err(|e| PiiError::StoreOp {
                op: String::from("begin_write"),
                resource: String::from("heal_attempt"),
                detail: format!("{e}"),
            })?;
            write
                .open_table(HEAL_ATTEMPT)
                .map_err(|e| PiiError::StoreOp {
                    op: String::from("open"),
                    resource: String::from("heal_table"),
                    detail: format!("{e}"),
                })?;
            write.commit().map_err(|e| PiiError::StoreOp {
                op: String::from("commit"),
                resource: String::from("heal_table"),
                detail: format!("{e}"),
            })?;
        }
        Ok(Self { db })
    }

    /// Inserts a new attempt row or updates an existing signature (S0.1).
    ///
    /// # Errors
    /// [`PiiError::StoreOp`] on a redb or encode failure.
    pub fn put_attempt(&self, a: &HealAttempt) -> Result<(), PiiError> {
        let bytes = to_bytes(a)?;
        let write = self.db.begin_write().map_err(|e| PiiError::StoreOp {
            op: String::from("begin_write"),
            resource: String::from("heal_attempt"),
            detail: format!("{e}"),
        })?;
        {
            let mut t = write
                .open_table(HEAL_ATTEMPT)
                .map_err(|e| PiiError::StoreOp {
                    op: String::from("open"),
                    resource: String::from("heal_table"),
                    detail: format!("{e}"),
                })?;
            t.insert(a.signature_hash.as_str(), bytes.as_slice())
                .map_err(|e| PiiError::StoreOp {
                    op: String::from("insert"),
                    resource: String::from("heal_attempt"),
                    detail: format!("{e}"),
                })?;
        }
        write.commit().map_err(|e| PiiError::StoreOp {
            op: String::from("commit"),
            resource: String::from("heal_attempt"),
            detail: format!("{e}"),
        })
    }

    /// Reads one attempt by signature hash (`None` when unknown).
    ///
    /// # Errors
    /// [`PiiError::StoreOp`] on a redb or decode failure.
    pub fn get_attempt(&self, sig: &str) -> Result<Option<HealAttempt>, PiiError> {
        let read = self.db.begin_read().map_err(|e| PiiError::StoreOp {
            op: String::from("begin_read"),
            resource: String::from("heal_attempt"),
            detail: format!("{e}"),
        })?;
        let t = read
            .open_table(HEAL_ATTEMPT)
            .map_err(|e| PiiError::StoreOp {
                op: String::from("open"),
                resource: String::from("heal_table"),
                detail: format!("{e}"),
            })?;
        let Some(v) = t.get(sig).map_err(|e| PiiError::StoreOp {
            op: String::from("get"),
            resource: String::from("heal_attempt"),
            detail: format!("{e}"),
        })?
        else {
            return Ok(None);
        };
        Ok(Some(from_bytes(v.value())?))
    }

    /// Every recorded attempt, in signature order.
    ///
    /// # Errors
    /// [`PiiError::StoreOp`] on a redb or decode failure.
    pub fn list_attempts(&self) -> Result<Vec<HealAttempt>, PiiError> {
        let read = self.db.begin_read().map_err(|e| PiiError::StoreOp {
            op: String::from("begin_read"),
            resource: String::from("heal_attempt"),
            detail: format!("{e}"),
        })?;
        let t = read
            .open_table(HEAL_ATTEMPT)
            .map_err(|e| PiiError::StoreOp {
                op: String::from("open"),
                resource: String::from("heal_table"),
                detail: format!("{e}"),
            })?;
        let mut out = Vec::new();
        for row in t.iter().map_err(|e| PiiError::StoreOp {
            op: String::from("iter"),
            resource: String::from("heal_attempt"),
            detail: format!("{e}"),
        })? {
            let row = row.map_err(|e| PiiError::StoreOp {
                op: String::from("row"),
                resource: String::from("heal_attempt"),
                detail: format!("{e}"),
            })?;
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
    /// [`PiiError::StoreOp`] on a redb or encode failure.
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
            gate_before: None,
            gate_after: None,
            decision: None,
            model: None,
            tokens_in: 0,
            tokens_out: 0,
            extra: serde_json::Map::new(),
        });
        existing.last_seen_ms = run.ts_ms;
        existing.attempt_count = existing.attempt_count.saturating_add(1);
        // R3 (spec 31): the top-level state tracks the MOST RECENT run's
        // outcome — e.g. a heal-collect refusal recorded with
        // `HealState::Escalated` moves the whole attempt to Escalated, not
        // just the individual run row.
        existing.state = run.outcome;
        existing.attempts.push(run);
        while existing.attempts.len() > ATTEMPT_RING_CAP {
            existing.attempts.remove(0);
        }
        self.put_attempt(&existing)?;
        Ok(existing)
    }
}

impl HealStore {
    /// Records a repair attempt's run AND the evidence fields that go with it
    /// (spec 33 R4): `gate_before`/`gate_after`/`patch_hash`/`model`/
    /// `tokens_in`/`tokens_out`. [`Self::record_run`] alone never populated
    /// these — this is the first producer. Reuses `record_run` for the run
    /// append + `attempt_count`/`state` bookkeeping, then patches the extra
    /// fields (a present `Some` overwrites; `None` leaves the prior value;
    /// token counts accumulate across attempts).
    ///
    /// # Errors
    /// [`PiiError::StoreOp`] on a redb or encode/decode failure.
    #[allow(clippy::too_many_arguments)] // reason: one evidence row, no
    // natural sub-grouping that wouldn't just be re-destructured downstream.
    pub fn record_repair(
        &self,
        sig: &str,
        gate: &str,
        run: HealAttemptRun,
        gate_before: Option<String>,
        gate_after: Option<String>,
        patch_hash: Option<String>,
        model: Option<String>,
        tokens_in: u64,
        tokens_out: u64,
    ) -> Result<HealAttempt, PiiError> {
        let mut attempt = self.record_run(sig, gate, run)?;
        if gate_before.is_some() {
            attempt.gate_before = gate_before;
        }
        if gate_after.is_some() {
            attempt.gate_after = gate_after;
        }
        if patch_hash.is_some() {
            attempt.patch_hash = patch_hash;
        }
        if model.is_some() {
            attempt.model = model;
        }
        attempt.tokens_in = attempt.tokens_in.saturating_add(tokens_in);
        attempt.tokens_out = attempt.tokens_out.saturating_add(tokens_out);
        self.put_attempt(&attempt)?;
        Ok(attempt)
    }
}

impl HealStore {
    /// Records the human decision on a promotion/heal-collect gate (R4, spec
    /// 34), bound to the `approval_id` the `gate` node minted. Creates the
    /// attempt row if `sig` has none yet — a `deny` can be the FIRST record
    /// for a signature that never went through [`Self::record_run`] (the
    /// promotion path records the decision independent of any prior
    /// detection/repair run). Always sets `decision`, whether `allow` or
    /// `deny` — the ledger must show a denied promotion just as durably as
    /// an approved one.
    ///
    /// # Errors
    /// [`PiiError::StoreOp`] on a redb or encode/decode failure.
    pub fn record_decision(
        &self,
        sig: &str,
        gate: &str,
        decision: ApprovalDecision,
    ) -> Result<HealAttempt, PiiError> {
        let mut attempt = self.get_attempt(sig)?.unwrap_or_else(|| HealAttempt {
            signature_hash: sig.to_owned(),
            gate: gate.to_owned(),
            state: HealState::Detected,
            first_seen_ms: decision.ts_ms,
            last_seen_ms: decision.ts_ms,
            attempts: Vec::new(),
            attempt_count: 0,
            patch_hash: None,
            gate_before: None,
            gate_after: None,
            decision: None,
            model: None,
            tokens_in: 0,
            tokens_out: 0,
            extra: serde_json::Map::new(),
        });
        attempt.last_seen_ms = decision.ts_ms;
        attempt.decision = Some(decision);
        self.put_attempt(&attempt)?;
        Ok(attempt)
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

    use super::{ATTEMPT_RING_CAP, ApprovalDecision, HealAttemptRun, HealState, HealStore};

    #[test]
    fn put_get_list_roundtrip() {
        let dir = Builder::new()
            .prefix("heal-store-")
            .tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp"))
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
            .tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp"))
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

    /// R3 (spec 31): recording a run with `HealState::Escalated` (e.g. a
    /// `heal-collect` refusal) moves the ATTEMPT's top-level state to
    /// Escalated, with the refusal reason readable back from the run's
    /// `detail` — not just recorded on the individual run row.
    #[test]
    fn heal_escalated_run_moves_top_level_state() {
        let dir = Builder::new()
            .prefix("heal-escalate-")
            .tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp"))
            .unwrap();
        let store = HealStore::open(&dir.path().join("heal.redb")).unwrap();
        let detail = String::from(
            "heal-collect: refused — specs/31-protected-paths.md touches a protected path",
        );
        let run = HealAttemptRun {
            run_id: String::from("r1"),
            outcome: HealState::Escalated,
            ts_ms: 100,
            detail: detail.clone(),
        };
        let a = store
            .record_run("sig-escalated", "heal-collect", run)
            .unwrap();
        assert_eq!(a.state, HealState::Escalated);
        assert_eq!(
            a.attempts.last().map(|r| r.detail.as_str()),
            Some(detail.as_str())
        );
        // Round-trips through storage, not just the in-memory return value.
        let reloaded = store.get_attempt("sig-escalated").unwrap().unwrap();
        assert_eq!(reloaded.state, HealState::Escalated);
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

    /// Spec 33 R4: `record_repair` sets the evidence fields `record_run`
    /// alone never touches, and accumulates token counts across attempts.
    #[test]
    fn record_repair_sets_evidence_fields() {
        let dir = Builder::new()
            .prefix("heal-repair-")
            .tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp"))
            .unwrap();
        let store = HealStore::open(&dir.path().join("heal.redb")).unwrap();
        let run = HealAttemptRun {
            run_id: String::from("r1"),
            outcome: HealState::Fixed,
            ts_ms: 100,
            detail: String::from("diff"),
        };
        let a = store
            .record_repair(
                "sig-repair",
                "heal-fix",
                run,
                Some(String::from("fail")),
                Some(String::from("pass")),
                Some(String::from("patchhash")),
                Some(String::from("modelX")),
                10,
                20,
            )
            .unwrap();
        assert_eq!(a.state, HealState::Fixed);
        assert_eq!(a.gate_before, Some(String::from("fail")));
        assert_eq!(a.gate_after, Some(String::from("pass")));
        assert_eq!(a.patch_hash, Some(String::from("patchhash")));
        assert_eq!(a.model, Some(String::from("modelX")));
        assert_eq!(a.tokens_in, 10);
        assert_eq!(a.tokens_out, 20);
        // Round-trips through storage.
        let reloaded = store.get_attempt("sig-repair").unwrap().unwrap();
        assert_eq!(reloaded.gate_before, Some(String::from("fail")));
        assert_eq!(reloaded.tokens_in, 10);
    }

    /// R4 (spec 34): `record_decision` creates a fresh attempt row for a
    /// signature that never went through `record_run` — a promotion decision
    /// (allow or deny) is recorded on its own, not only as a follow-up to a
    /// prior detection/repair run.
    #[test]
    fn record_decision_creates_row_for_unknown_signature() {
        let dir = Builder::new()
            .prefix("heal-decision-")
            .tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp"))
            .unwrap();
        let store = HealStore::open(&dir.path().join("heal.redb")).unwrap();
        assert!(store.get_attempt("promo-sig").unwrap().is_none());

        let decision = ApprovalDecision {
            allow: true,
            approval_id: String::from("approval-abc"),
            by: String::from("op1"),
            ts_ms: 555,
        };
        let a = store
            .record_decision("promo-sig", "promotion", decision.clone())
            .unwrap();
        assert_eq!(a.decision, Some(decision));
        let reloaded = store.get_attempt("promo-sig").unwrap().unwrap();
        assert!(reloaded.decision.unwrap().allow);
    }

    /// R4 (spec 34): a DENY decision is recorded just as durably as an
    /// allow — `decision.allow == false` round-trips through storage. This
    /// is the ledger half of T3 (spec 34's acceptance test): a denied
    /// promotion must be visible in the ledger even though it produces no
    /// patch artifact.
    #[test]
    fn record_decision_deny_round_trips() {
        let dir = Builder::new()
            .prefix("heal-decision-deny-")
            .tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp"))
            .unwrap();
        let store = HealStore::open(&dir.path().join("heal.redb")).unwrap();
        let decision = ApprovalDecision {
            allow: false,
            approval_id: String::from("approval-deny"),
            by: String::from("op2"),
            ts_ms: 42,
        };
        store
            .record_decision("deny-sig", "promotion", decision)
            .unwrap();
        let reloaded = store.get_attempt("deny-sig").unwrap().unwrap();
        assert!(!reloaded.decision.unwrap().allow);
    }

    /// R1/R4 (spec 46): a fresh `HealAttempt` built via `record_run` on a
    /// brand-new signature has an empty `extra` bag.
    #[test]
    fn heal_attempt_extra_defaults_to_empty() {
        let dir = Builder::new()
            .prefix("heal-extra-")
            .tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp"))
            .unwrap();
        let store = HealStore::open(&dir.path().join("heal.redb")).unwrap();
        let run = HealAttemptRun {
            run_id: String::from("r1"),
            outcome: HealState::Detected,
            ts_ms: 100,
            detail: String::from("gate failed"),
        };
        let a = store.record_run("extra-sig", "quality-gate", run).unwrap();
        assert!(a.extra.is_empty());
        let reloaded = store.get_attempt("extra-sig").unwrap().unwrap();
        assert!(reloaded.extra.is_empty());
    }

    /// R5 (spec 46): an old-shape encoded payload with the removed
    /// `worktree`/`branch`/`base_commit`/`micro_usd` fields still present
    /// must decode cleanly into the new `HealAttempt` shape, with `extra`
    /// defaulting to empty. Proves the JSON codec's structural (not
    /// positional) tolerance of removed fields.
    #[test]
    fn heal_attempt_old_shape_deserializes_with_removed_fields_present() {
        use super::HealAttempt;
        let json = r#"{
            "signature_hash": "old-shape",
            "gate": "quality-gate",
            "state": "detected",
            "first_seen_ms": 1,
            "last_seen_ms": 2,
            "attempts": [],
            "attempt_count": 3,
            "patch_hash": "somehash",
            "worktree": "/tmp/wt-123",
            "branch": "heal/fix-abc",
            "base_commit": "deadbeef",
            "gate_before": "fail",
            "gate_after": "pass",
            "decision": null,
            "model": "modelX",
            "tokens_in": 10,
            "tokens_out": 20,
            "micro_usd": 500
        }"#;
        let row: HealAttempt =
            serde_json::from_str(json).expect("old-shape row must decode without error");
        assert_eq!(row.signature_hash, "old-shape");
        assert_eq!(row.patch_hash, Some(String::from("somehash")));
        assert_eq!(row.tokens_in, 10);
        assert_eq!(row.tokens_out, 20);
        assert!(
            row.extra.is_empty(),
            "extra must be empty when decoding an old-shape row"
        );
    }
}
