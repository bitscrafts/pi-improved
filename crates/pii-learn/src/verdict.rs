//! The verifier's typed, honest verdicts and where they are kept (spec 124 R2,
//! R4, R6): `<dir>/learn/verdicts/<candidate id>.json` plus
//! `<dir>/learn/verify_status.json`. These are the only files the verifier writes.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::candidate::is_valid_id;
use crate::store::{StoreError, area, read_small, write_atomic};

/// What a shadow run decided about a claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// The claim reproduced in the shadow run's own numbers.
    Confirmed,
    /// The claim did not reproduce. A successful verification, not a failure.
    Refuted,
    /// Undecided; `reason` says why. Never a refutation.
    Inconclusive,
}

impl Verdict {
    /// Every verdict, in report order.
    pub const ALL: [Self; 3] = [Self::Confirmed, Self::Refuted, Self::Inconclusive];

    /// The stable `snake_case` name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Confirmed => "confirmed",
            Self::Refuted => "refuted",
            Self::Inconclusive => "inconclusive",
        }
    }
}

/// Why a verdict is `inconclusive` (a closed set; it names the bound or path).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// Cited artefacts no longer exist (spec 115 retention): absence is not refutation (R5).
    EvidenceMissing,
    /// Cited artefacts exist but cannot be read or are outside the root.
    EvidenceUnreadable,
    /// Needs more shadow runs than `max_shadow_runs` allows (R3).
    RunCap,
    /// A shadow run exceeded `shadow_run_ms` (R3).
    Timeout,
    /// The claim is about limits or cost, which a re-run cannot test.
    NotTestable,
    /// No graph spec of that name to re-execute.
    GraphUnavailable,
    /// Isolation could not be guaranteed (guardrail): no best-effort run.
    Isolation,
    /// The engine failed to run at all (not a failure *of the graph*).
    EngineError,
}

impl Reason {
    /// The stable `snake_case` name carried by `learn.candidate_inconclusive`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::EvidenceMissing => "evidence_missing",
            Self::EvidenceUnreadable => "evidence_unreadable",
            Self::RunCap => "run_cap",
            Self::Timeout => "timeout",
            Self::NotTestable => "not_testable",
            Self::GraphUnavailable => "graph_unavailable",
            Self::Isolation => "isolation",
            Self::EngineError => "engine_error",
        }
    }
}

/// One candidate's verdict, with the evidence used and the shadow run's id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerdictRecord {
    /// The candidate this decides.
    pub candidate: String,
    /// The verdict.
    pub verdict: Verdict,
    /// Set exactly when `verdict` is `inconclusive`.
    pub reason: Option<Reason>,
    /// One line: the shadow run's numbers, or the bound / missing paths.
    pub detail: String,
    /// The shadow session id (`s-<candidate hash>`); empty when nothing ran.
    pub shadow: String,
    /// Shadow run ids that actually executed (`<shadow>/<run>`).
    pub shadow_runs: Vec<String>,
    /// The shadow run's own numbers (`shadow_runs`, `shadow_failed`, ...).
    pub numbers: BTreeMap<String, u64>,
    /// The artefact paths consulted.
    pub evidence_used: Vec<String>,
    /// Cited paths that no longer exist (R5).
    pub missing: Vec<String>,
    /// The bound that was hit, e.g. `max_shadow_runs=2`.
    pub bound: Option<String>,
    /// Wall-clock milliseconds the verification took.
    pub elapsed_ms: u64,
}

/// The last verification pass, persisted for `pii health` / `pii metrics`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct VerifyStatus {
    /// Seconds since the Unix epoch when the pass ended.
    pub at_secs: u64,
    /// Candidates verified in the pass.
    pub candidates: u64,
    /// Verdict counts of the pass (all three keys present).
    pub verdicts: BTreeMap<String, u64>,
}

fn verdicts_dir(dir: &Path) -> std::path::PathBuf {
    area(dir).join("verdicts")
}

/// Records `v` keyed by its candidate id.
///
/// # Errors
/// [`StoreError`] when the id is malformed or the file cannot be written.
pub fn put_verdict(dir: &Path, v: &VerdictRecord) -> Result<(), StoreError> {
    if !is_valid_id(&v.candidate) {
        return Err(StoreError(format!("not a candidate id: {}", v.candidate)));
    }
    let vdir = verdicts_dir(dir);
    fs::create_dir_all(&vdir).map_err(|e| StoreError(format!("create {}: {e}", vdir.display())))?;
    let bytes = serde_json::to_vec_pretty(v).map_err(|e| StoreError(e.to_string()))?;
    write_atomic(&vdir.join(format!("{}.json", v.candidate)), &bytes)
}

/// The recorded verdict for `id`, if any.
#[must_use]
pub fn get_verdict(dir: &Path, id: &str) -> Option<VerdictRecord> {
    if !is_valid_id(id) {
        return None;
    }
    let bytes = read_small(&verdicts_dir(dir).join(format!("{id}.json")))?;
    serde_json::from_slice(&bytes).ok()
}

/// Persists the last pass's status.
///
/// # Errors
/// [`StoreError`] when the file cannot be written.
pub fn write_verify_status(dir: &Path, s: &VerifyStatus) -> Result<(), StoreError> {
    fs::create_dir_all(area(dir)).map_err(|e| StoreError(e.to_string()))?;
    let bytes = serde_json::to_vec_pretty(s).map_err(|e| StoreError(e.to_string()))?;
    write_atomic(&area(dir).join("verify_status.json"), &bytes)
}

/// The last pass's status; `None` before any pass or when unreadable.
#[must_use]
pub fn read_verify_status(dir: &Path) -> Option<VerifyStatus> {
    let bytes = read_small(&area(dir).join("verify_status.json"))?;
    serde_json::from_slice(&bytes).ok()
}
