//! The run outcome: the engine's terminal, carried verbatim.

use serde_json::Value;

use crate::{Event, Level, names};

/// How a run ended. Mirrors the engine's terminal dispositions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Verdict {
    /// The run succeeded.
    Success,
    /// The run failed.
    Failure,
    /// The run was cancelled.
    Cancelled,
    /// The run suspended and can be resumed.
    Suspended,
}

impl Verdict {
    /// The stable lowercase wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
            Self::Cancelled => "cancelled",
            Self::Suspended => "suspended",
        }
    }
}

/// A run's terminal, verbatim: the verdict, a short reason, and the engine's
/// own payload (`detail`: the output, the error text, or the resume key).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// How the run ended.
    pub verdict: Verdict,
    /// Short machine-readable reason (e.g. `"terminal"`).
    pub reason: String,
    /// The terminal payload exactly as the engine produced it.
    pub detail: Value,
}

impl Outcome {
    /// The `run.outcome` event carrying this outcome. `Failure` is logged at
    /// `Error`, everything else at `Info`.
    #[must_use]
    pub fn into_event(self, subsystem: &'static str) -> Event {
        let level = if self.verdict == Verdict::Failure {
            Level::Error
        } else {
            Level::Info
        };
        Event::new(subsystem, level, names::RUN_OUTCOME)
            .field("verdict", self.verdict.as_str())
            .field("reason", self.reason)
            .field("detail", self.detail)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn outcome_event_carries_verdict_reason_and_detail_verbatim() {
        let e = Outcome {
            verdict: Verdict::Failure,
            reason: "terminal".into(),
            detail: serde_json::json!("boom: exit 3"),
        }
        .into_event("graph");
        assert_eq!(e.kind, names::RUN_OUTCOME);
        assert_eq!(e.level, Level::Error);
        assert_eq!(e.fields.get("verdict"), Some(&Value::from("failure")));
        assert_eq!(e.fields.get("detail"), Some(&Value::from("boom: exit 3")));
    }
}
