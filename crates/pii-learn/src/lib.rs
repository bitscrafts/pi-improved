//! `pii-learn` (spec 123): the learning loop's first stage — a deterministic,
//! offline reader that turns the trajectory substrate into **cited candidate
//! lessons**. It proposes; it never acts. Stage 124 verifies, 125 promotes.
//!
//! Why its own crate: it needs only `pii-observe` (the trajectory format and
//! names), so it stays off the engine's dependency path, and the daemon links
//! it solely for the read-only `learn` health seam. No model call, no network.
//!
//! See `_docs/02-guides/learning-loop.md`.

#![forbid(unsafe_code)]

mod candidate;
mod config;
pub mod distance;
mod extract;
pub mod incident;
pub mod incident_runtime;
pub mod incident_store;
mod learner;
pub mod loop_events;
mod promote;
mod scan;
pub mod store;
mod unattended;
mod verdict;
mod verify;

pub use candidate::{Candidate, Evidence, Kind, Refusal, is_valid_id};
pub use config::{LearnConfig, MAX_EVIDENCE_RUNS};
pub use extract::{Extraction, extract_all};
pub use learner::{LearnHealth, Learner};
pub use promote::{
    DEFAULT_TTL_SECS, DefinitionHash, LedgerEntry, Metric, Outcome, PromoteConfig, PromoteRequest,
    Promoter, Promotion, PromotionKind, PromotionState, Refusal as PromotionRefusal, Refused,
    Rollback, get_promotion, ledger_id, ledger_tail, list_promotions, measure, promotion_counts,
    quarantine_reason, quarantine_refusal, quarantining_promotion, write_scoped,
};
pub use scan::{RunSummary, ScanError, ScanStats, classify, scan, token};
pub use store::{LearnStatus, StoreError};
pub use unattended::{UnattendedConfig, UnattendedStatus, read_unattended_status, run_unattended};
pub use verdict::{
    Reason, Verdict, VerdictRecord, VerifyStatus, get_verdict, put_verdict, read_verify_status,
    write_verify_status,
};
pub use verify::{DEFAULT_MAX_SHADOW_RUNS, DEFAULT_SHADOW_RUN_MS, Verifier, VerifyConfig};
