//! Context compaction policy (R3).
//!
//! Decides when to compact, and summarizes by decisions and outcomes,
//! never dialogue verbatim (the Anthropic/Zylos guidance this crate
//! follows). Compaction fires at ~70-75% of the model's context window,
//! not 95%, so a long-running boss survives ten or more compactions over a
//! session without losing its footing. `state.md` (see [`crate::state`])
//! remains the "where am I" source of truth; this module only governs the
//! transcript, and durable facts still go to external memory, never into a
//! compaction summary.

/// Compaction knobs: fire once transcript usage crosses `threshold` of
/// `window_tokens`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompactionPolicy {
    /// The model's context window, in tokens.
    pub window_tokens: usize,
    /// Fire compaction once usage crosses this fraction of the window.
    pub threshold: f64,
}

impl Default for CompactionPolicy {
    /// ~72% of the window (Zylos guidance: 70-75%, not 95%).
    fn default() -> Self {
        Self {
            window_tokens: 200_000,
            threshold: 0.72,
        }
    }
}

impl CompactionPolicy {
    /// True once `used_tokens` crosses `threshold` of `window_tokens`.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    // reason: token counts never approach f64's 2^53 exact-integer limit;
    // the loss is immaterial to a fraction-of-window comparison.
    pub fn should_compact(&self, used_tokens: usize) -> bool {
        (used_tokens as f64) > (self.window_tokens as f64) * self.threshold
    }
}

/// One entry in a compactable transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranscriptEntry {
    /// Free-form conversation text — dropped from the compaction summary.
    Dialogue(String),
    /// A decision and its outcome — the only thing compaction keeps. This
    /// is the in-context echo of a fact that also belongs in `state.md` or
    /// external memory, not a replacement for either.
    Decision {
        /// What was decided.
        decision: String,
        /// What happened as a result.
        outcome: String,
    },
}

/// Builds the compaction summary for `entries`: every
/// [`TranscriptEntry::Decision`] renders as `decision -> outcome`; every
/// [`TranscriptEntry::Dialogue`] entry is dropped.
#[must_use]
pub fn summarize_for_compaction(entries: &[TranscriptEntry]) -> String {
    let mut out = String::from("Compacted context (decisions and outcomes):\n");
    for entry in entries {
        if let TranscriptEntry::Decision { decision, outcome } = entry {
            out.push_str("- ");
            out.push_str(decision);
            out.push_str(" -> ");
            out.push_str(outcome);
            out.push('\n');
        }
    }
    out
}
