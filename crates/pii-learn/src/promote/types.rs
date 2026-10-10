//! The promotion vocabulary (spec 125): kinds, states, refusals, records.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::candidate::{Candidate, Evidence, Kind};
use crate::verdict::Verdict;

/// Default time a promotion lasts before it lapses (R4): 24 hours.
pub const DEFAULT_TTL_SECS: u64 = 24 * 60 * 60;

/// What a promotion does. This enum IS the whitelist (R1): a new kind is one
/// variant plus a match arm in each function below, and needs its own spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromotionKind {
    /// The daemon refuses to start the named graph.
    QuarantineGraph,
}

impl PromotionKind {
    /// Every whitelisted kind.
    pub const ALL: [Self; 1] = [Self::QuarantineGraph];

    /// The stable `snake_case` name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::QuarantineGraph => "quarantine_graph",
        }
    }

    /// Parses the wire name once; anything off the whitelist is refused (R6).
    ///
    /// # Errors
    /// The named refusal text for a kind that is not whitelisted.
    pub fn parse(s: &str) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|k| k.as_str() == s)
            .ok_or_else(|| {
                format!("learn: promotion kind {s} is not whitelisted (only quarantine_graph)")
            })
    }

    /// The graph this kind of promotion targets, derived from the candidate.
    pub(super) fn target(self, c: &Candidate) -> Option<String> {
        match self {
            Self::QuarantineGraph => match c.kind {
                Kind::RepeatedFailure | Kind::StuckLoop | Kind::CostOutlier => {
                    c.dedupe_key.split(':').nth(1).map(str::to_owned)
                }
                Kind::LimitHit => None,
            }
            .filter(|g| valid_graph(g)),
        }
    }

    /// The state to restore, captured at apply time (R4).
    pub(super) fn rollback(self, graph: &str, now: u64) -> Rollback {
        match self {
            Self::QuarantineGraph => Rollback {
                kind: self,
                graph: graph.to_owned(),
                was_quarantined: false,
                captured_secs: now,
            },
        }
    }
}

/// The metric a promotion declares it will move (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Metric {
    /// Failed runs of the promoted graph in a window; lower is better.
    GraphFailedRuns,
}

impl Metric {
    /// The stable `snake_case` name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::GraphFailedRuns => "graph_failed_runs",
        }
    }

    /// Parses the wire name.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        (s == Self::GraphFailedRuns.as_str()).then_some(Self::GraphFailedRuns)
    }
}

/// Where a promotion is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromotionState {
    /// In effect.
    Active,
    /// Taken back by a person.
    Reverted,
    /// Lapsed at its TTL.
    Expired,
    /// Taken back by the daemon because the metric did not improve.
    AutoReverted,
}

impl PromotionState {
    /// Every state, in report order.
    pub const ALL: [Self; 4] = [
        Self::Active,
        Self::Reverted,
        Self::Expired,
        Self::AutoReverted,
    ];

    /// The stable `snake_case` name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Reverted => "reverted",
            Self::Expired => "expired",
            Self::AutoReverted => "auto_reverted",
        }
    }
}

/// Why a promotion was refused (a closed set; counted by name).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Refusal {
    /// The kind is not on the whitelist.
    UnknownKind,
    /// No such candidate.
    NoCandidate,
    /// The candidate has no verdict.
    NoVerdict,
    /// The verdict is `refuted`.
    Refuted,
    /// The verdict is `inconclusive`.
    Inconclusive,
    /// No metric or no window: the effect could not be measured.
    Unmeasurable,
    /// The candidate names no graph this kind can target.
    NoGraph,
    /// An active promotion already covers that kind and graph.
    AlreadyActive,
    /// Rollback data is missing; nothing is applied without it.
    NoRollback,
    /// A write would leave the promotion's own store area.
    Scope,
    /// The TTL is zero.
    BadTtl,
    /// The ledger or the record could not be written.
    Store,
}

impl Refusal {
    /// Every reason, in report order.
    pub const ALL: [Self; 12] = [
        Self::UnknownKind,
        Self::NoCandidate,
        Self::NoVerdict,
        Self::Refuted,
        Self::Inconclusive,
        Self::Unmeasurable,
        Self::NoGraph,
        Self::AlreadyActive,
        Self::NoRollback,
        Self::Scope,
        Self::BadTtl,
        Self::Store,
    ];

    /// The stable `snake_case` name carried by `learn.promotion_refused`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnknownKind => "unknown_kind",
            Self::NoCandidate => "no_candidate",
            Self::NoVerdict => "no_verdict",
            Self::Refuted => "refuted",
            Self::Inconclusive => "inconclusive",
            Self::Unmeasurable => "unmeasurable",
            Self::NoGraph => "no_graph",
            Self::AlreadyActive => "already_active",
            Self::NoRollback => "no_rollback",
            Self::Scope => "scope",
            Self::BadTtl => "bad_ttl",
            Self::Store => "store",
        }
    }
}

/// A refused promotion: the reason, the text, and the ledger id of the record
/// of the refusal when one could be written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    /// The closed reason.
    pub reason: Refusal,
    /// Human text naming the reason.
    pub message: String,
    /// The ledger id of the refusal record, if written.
    pub ledger: Option<String>,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.reason.as_str(), self.message)
    }
}

/// What to restore when a promotion ends (R4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rollback {
    /// The kind that was applied.
    pub kind: PromotionKind,
    /// The graph it applied to.
    pub graph: String,
    /// Whether the graph was already quarantined before this promotion.
    pub was_quarantined: bool,
    /// Epoch seconds the previous state was captured.
    pub captured_secs: u64,
}

/// The measured outcome of a promotion's window (R5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outcome {
    /// The metric over the window before the promotion.
    pub before: u64,
    /// The metric over the window after it.
    pub after: u64,
    /// `after < before`.
    pub improved: bool,
}

/// One promotion, as stored under `<dir>/learn/promotions/<id>.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Promotion {
    /// `p-` plus 16 hex.
    pub id: String,
    /// What it does.
    pub kind: PromotionKind,
    /// The graph it targets.
    pub graph: String,
    /// The candidate it rests on.
    pub candidate: String,
    /// The verdict re-read at promotion time.
    pub verdict: Verdict,
    /// The candidate's evidence.
    pub evidence: Evidence,
    /// Who or what asked.
    pub actor: String,
    /// Where it is in its life.
    pub state: PromotionState,
    /// What to restore; a promotion without it is never in effect.
    pub rollback: Option<Rollback>,
    /// Epoch seconds applied.
    pub applied_secs: u64,
    /// How long it lasts.
    pub ttl_secs: u64,
    /// `applied_secs + ttl_secs`.
    pub expires_secs: u64,
    /// The metric it declared.
    pub metric: Metric,
    /// The measurement window, seconds.
    pub window_secs: u64,
    /// The window's result once evaluated.
    pub outcome: Option<Outcome>,
    /// Epoch seconds it left `active`.
    pub ended_secs: Option<u64>,
    /// The decision, in words.
    pub decision: String,
}

impl Promotion {
    /// Whether it is in effect at `now`: active, unexpired, with rollback data.
    #[must_use]
    pub fn in_effect(&self, now: u64) -> bool {
        self.state == PromotionState::Active && now < self.expires_secs && self.rollback.is_some()
    }
}

/// What a caller asks for.
#[derive(Debug, Clone)]
pub struct PromoteRequest {
    /// The candidate to promote; its verdict is re-read, never passed in.
    pub candidate: String,
    /// The kind's wire name; parsed against the whitelist.
    pub kind: String,
    /// Who or what asked (named).
    pub actor: String,
    /// The metric's wire name, if declared.
    pub metric: Option<String>,
    /// The window in seconds, if declared.
    pub window_secs: Option<u64>,
    /// Overrides [`PromoteConfig::ttl_secs`].
    pub ttl_secs: Option<u64>,
}

/// Promotion settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromoteConfig {
    /// Default TTL, seconds.
    pub ttl_secs: u64,
}

impl Default for PromoteConfig {
    fn default() -> Self {
        Self {
            ttl_secs: DEFAULT_TTL_SECS,
        }
    }
}

/// One line of the decision ledger (`<dir>/learn/ledger.jsonl`, append-only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerEntry {
    /// Line number, from 1; the ledger id is `L<seq>`.
    pub seq: u64,
    /// Epoch seconds.
    pub at_secs: u64,
    /// `applied`, `reverted`, `expired`, `auto_reverted`, `outcome_kept`, `refused`.
    pub event: String,
    /// The promotion (empty for a refusal before one existed).
    pub promotion: String,
    /// The kind asked for.
    pub kind: String,
    /// The graph, when known.
    pub graph: String,
    /// The candidate.
    pub candidate: String,
    /// The verdict at the time, if any.
    pub verdict: Option<Verdict>,
    /// The candidate's evidence, if it was readable.
    pub evidence: Option<Evidence>,
    /// Who or what asked.
    pub actor: String,
    /// State before the transition.
    pub from: Option<PromotionState>,
    /// State after (`None` for a refusal).
    pub to: Option<PromotionState>,
    /// Declared metric, if any.
    pub metric: Option<String>,
    /// Metric before / after, once measured.
    pub before: Option<u64>,
    /// See `before`.
    pub after: Option<u64>,
    /// The decision, in words.
    pub decision: String,
}

/// `L<seq>`: the id printed by every command.
#[must_use]
pub fn ledger_id(seq: u64) -> String {
    format!("L{seq}")
}

/// Counters persisted beside the promotions (refusals leave no promotion).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub(super) struct PromotionStatus {
    pub(super) refused_by_reason: BTreeMap<String, u64>,
}

pub(super) fn valid_graph(g: &str) -> bool {
    !g.is_empty()
        && g.len() <= 64
        && !g.starts_with('.')
        && g.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-')
}
