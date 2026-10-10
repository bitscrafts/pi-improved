//! The candidate lesson: a small typed record whose claim is derivable from
//! its cited evidence alone (spec 123 R1, R3).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::config::MAX_EVIDENCE_RUNS;

/// What a candidate says happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// The same node and error class failed across several runs.
    RepeatedFailure,
    /// A graph failed the same way in every run and never succeeded.
    StuckLoop,
    /// A named limit or refusal was hit in several runs.
    LimitHit,
    /// A run took far more time or bytes than the graph's median.
    CostOutlier,
}

impl Kind {
    /// Every kind, in report order.
    pub const ALL: [Self; 4] = [
        Self::RepeatedFailure,
        Self::StuckLoop,
        Self::LimitHit,
        Self::CostOutlier,
    ];

    /// The stable `snake_case` name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RepeatedFailure => "repeated_failure",
            Self::StuckLoop => "stuck_loop",
            Self::LimitHit => "limit_hit",
            Self::CostOutlier => "cost_outlier",
        }
    }
}

/// What a claim rests on: runs, artefact paths and the counts that triggered it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    /// Run ids (capped at [`MAX_EVIDENCE_RUNS`]; the true number is in `counts`).
    pub runs: Vec<String>,
    /// Trajectory artefact paths, parallel to `runs`.
    pub paths: Vec<String>,
    /// The concrete numbers that crossed the threshold.
    pub counts: BTreeMap<String, u64>,
}

/// A proposed lesson. It is only ever proposed; nothing acts on it here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    /// Content-derived: the same evidence yields the same id.
    pub id: String,
    /// What kind of observation.
    pub kind: Kind,
    /// One line, built from `evidence.counts` and closed-vocabulary tokens.
    pub claim: String,
    /// What the claim rests on.
    pub evidence: Evidence,
    /// Groups candidates about the same thing across scans.
    pub dedupe_key: String,
}

/// Why a candidate was refused instead of emitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Refusal {
    /// No run id, or no artefact path.
    NoEvidence,
    /// No count: nothing says what crossed the threshold.
    NoCounts,
    /// An empty claim or dedupe key.
    NoClaim,
}

impl Refusal {
    /// The stable wire name carried by `learn.candidate_refused`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoEvidence => "no_evidence",
            Self::NoCounts => "no_counts",
            Self::NoClaim => "no_claim",
        }
    }
}

impl Candidate {
    /// Builds a candidate, or refuses it. Every extractor goes through here,
    /// so there is no path that emits an unevidenced claim (R3).
    ///
    /// # Errors
    /// [`Refusal`] when the evidence has no (non-blank) run id or artefact
    /// path, no counts, or the claim or dedupe key is blank.
    pub fn new(
        kind: Kind,
        claim: String,
        dedupe_key: String,
        mut evidence: Evidence,
    ) -> Result<Self, Refusal> {
        evidence.runs.truncate(MAX_EVIDENCE_RUNS);
        evidence.paths.truncate(MAX_EVIDENCE_RUNS);
        let has = |v: &[String]| !v.is_empty() && v.iter().all(|s| !s.trim().is_empty());
        if !has(&evidence.runs) || !has(&evidence.paths) {
            return Err(Refusal::NoEvidence);
        }
        if evidence.counts.is_empty() {
            return Err(Refusal::NoCounts);
        }
        if claim.trim().is_empty() || dedupe_key.trim().is_empty() {
            return Err(Refusal::NoClaim);
        }
        let id = derive_id(kind, &dedupe_key, &evidence);
        Ok(Self {
            id,
            kind,
            claim,
            evidence,
            dedupe_key,
        })
    }
}

/// `c-` plus 16 hex of blake3 over kind, key, runs and counts. Paths are left
/// out on purpose: moving a trajectory root must not rename a lesson.
fn derive_id(kind: Kind, key: &str, ev: &Evidence) -> String {
    let mut h = blake3::Hasher::new();
    h.update(kind.as_str().as_bytes());
    h.update(&[0]);
    h.update(key.as_bytes());
    for r in &ev.runs {
        h.update(&[0]);
        h.update(r.as_bytes());
    }
    for (k, v) in &ev.counts {
        h.update(&[1]);
        h.update(k.as_bytes());
        h.update(&v.to_le_bytes());
    }
    let hex = h.finalize().to_hex();
    format!("c-{}", hex.as_str().get(..16).unwrap_or(hex.as_str()))
}

/// Whether `id` has the shape [`derive_id`] produces (guards the store path).
#[must_use]
pub fn is_valid_id(id: &str) -> bool {
    id.strip_prefix("c-").is_some_and(|h| {
        h.len() == 16
            && h.bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    })
}
