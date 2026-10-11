//! Spec 131: a quarantine refusal is recorded as a deny decision.
//!
//! The decision ledger must show it, and the daemon owns the heal store, so
//! the record is written here, where the effect happens.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use pii_abi::RunId;
use pii_learn::Promotion;
use pii_store::heal::{ApprovalDecision, HealAttemptRun, HealState};

use crate::Engine;

/// The decision `gate` of a quarantine refusal: the *effect* of a promotion,
/// not an approval of one.
pub const QUARANTINE_GATE: &str = "quarantine";

/// Refusals vs refusals recorded (R4): a gap between them is a finding.
#[derive(Default)]
pub struct RefusalCounts {
    refused: AtomicU64,
    recorded: AtomicU64,
    /// Test instrumentation: while set, the ledger write fails.
    fail_record_for_test: AtomicBool,
}

impl RefusalCounts {
    /// Quarantine refusals issued since start.
    #[must_use]
    pub fn refused(&self) -> u64 {
        self.refused.load(Ordering::Relaxed)
    }

    /// Quarantine refusals whose deny decision reached the ledger.
    #[must_use]
    pub fn recorded(&self) -> u64 {
        self.recorded.load(Ordering::Relaxed)
    }

    /// **Test instrumentation only.** Makes the ledger write fail.
    pub fn set_fail_record_for_test(&self, fail: bool) {
        self.fail_record_for_test.store(fail, Ordering::Relaxed);
    }
}

/// Records the deny decision for a quarantine refusal. Never fails the
/// refusal (R4): a write error is reported out-of-band and left as a gap
/// between the two counters. One write, no retry.
pub fn record_quarantine_refusal(
    engine: &Engine,
    promo: &Promotion,
    reason: &str,
    run_id: &RunId,
    now_ms: u64,
) {
    engine.refusals.refused.fetch_add(1, Ordering::Relaxed);
    // Join key: `sig` is the PROMOTION id (the record that caused the denial,
    // `pii learn show`-visible); `approval_id` stays empty because a promotion
    // carries none and the run id is not an approval.
    let decision = ApprovalDecision {
        allow: false,
        approval_id: String::new(),
        by: String::from("engine"),
        ts_ms: now_ms,
    };
    let extra = serde_json::json!({
        "graph": promo.graph,
        "reason": reason,
        "run_id": run_id.as_str(),
    });
    let denied_run = HealAttemptRun {
        run_id: run_id.as_str().to_owned(),
        outcome: HealState::Denied,
        ts_ms: now_ms,
        detail: reason.to_owned(),
    };
    let serde_json::Value::Object(extra) = extra else {
        return;
    };
    let written = if engine.refusals.fail_record_for_test.load(Ordering::Relaxed) {
        Err(String::from("injected ledger fault"))
    } else {
        engine
            .heals
            .record_denial(&promo.id, QUARANTINE_GATE, decision, denied_run, extra)
            .map(|_| ())
            .map_err(|e| e.to_string())
    };
    match written {
        Ok(()) => {
            engine.refusals.recorded.fetch_add(1, Ordering::Relaxed);
        }
        Err(e) => engine.report_unhandled("quarantine.refusal.record", &e),
    }
}
