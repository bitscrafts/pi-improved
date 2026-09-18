//! Kernel error type.

use thiserror::Error;

use crate::failover::{ProviderFailure, Recovery, classify_provider};

/// Errors surfaced by the Pii kernel ABI.
///
/// `is_retryable` and `is_terminal` are mutually exclusive but NOT exhaustive:
/// a variant can be neither (e.g. `NotFound`, `Internal`), meaning it is
/// neither worth retrying nor guaranteed to never succeed. Do not route on
/// `!is_terminal()` as "retry this" — that would retry `Internal` forever.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PiiError {
    /// An input or parameter was not valid.
    #[error("invalid: {0}")]
    Invalid(String),
    /// An operation was denied by policy or capability.
    #[error("capability denied: {0}")]
    Denied(String),
    /// An operation was cancelled before completion.
    #[error("cancelled")]
    Cancelled,
    /// A deadline was exceeded.
    #[error("deadline exceeded after {after_ms}ms")]
    Timeout {
        /// The deadline in milliseconds that was missed.
        after_ms: u64,
    },
    /// A budget was exhausted.
    #[error("budget exhausted: {0}")]
    BudgetExhausted(String),
    /// A requested item was not found.
    #[error("not found: {0}")]
    NotFound(String),
    /// An unrecoverable internal fault.
    #[error("internal: {0}")]
    Internal(String),
    /// A transient dependency failure; the operation may succeed on retry
    /// (F1.2). Worker death and HTTP 429/5xx are its producers.
    #[error("unavailable: {0}")]
    Unavailable(String),
    /// A structured provider failure: the primitives `classify_provider`
    /// needs to reach a `FailoverReason`, not a formatted string.
    #[error("provider failure: {0:?}")]
    Provider(ProviderFailure),
    /// A structured store-operation failure (spec 47 R1 pilot): the redb
    /// operation (`op`) that failed, the resource it targeted, and the
    /// underlying error detail. Classification matches `Internal` — neither
    /// retryable nor terminal — because a store failure may be transient
    /// (I/O hiccup) or permanent (disk full); callers decide.
    #[error("store op {op} on {resource}: {detail}")]
    StoreOp {
        /// The store operation that failed (`open`, `begin_write`,
        /// `commit`, `insert`, `get`, `iter`, …).
        op: String,
        /// The resource the operation targeted (`heal_db`, `heal_attempt`, …).
        resource: String,
        /// The underlying error detail.
        detail: String,
    },
}

impl PiiError {
    /// True when re-running the failed operation is worth trying.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Timeout { .. } | Self::Unavailable(_) => true,
            Self::Provider(f) => classify_provider(f).recovery() == Recovery::RetrySame,
            _ => false,
        }
    }

    /// True when the operation can never succeed, so retrying is pointless.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Denied(_) | Self::Invalid(_) | Self::BudgetExhausted(_) | Self::Cancelled
        )
    }

    /// True when the operation was cancelled rather than failed.
    #[must_use]
    pub const fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::PiiError;
    use crate::failover::{Origin, ProviderFailure};

    #[test]
    fn pii_error_display() {
        let err = PiiError::Denied(String::from("fs_write"));
        assert!(err.to_string().contains("fs_write"));
    }

    #[test]
    fn pii_error_retryable_and_terminal_partition() {
        let variants = [
            PiiError::Invalid(String::new()),
            PiiError::Denied(String::new()),
            PiiError::Cancelled,
            PiiError::Timeout { after_ms: 1 },
            PiiError::BudgetExhausted(String::new()),
            PiiError::NotFound(String::new()),
            PiiError::Internal(String::new()),
            PiiError::StoreOp {
                op: String::from("commit"),
                resource: String::from("heal_attempt"),
                detail: String::from("disk full"),
            },
            PiiError::Provider(ProviderFailure {
                status: Some(500),
                origin: Origin::Provider,
                provider: String::from("ollama"),
                message: String::new(),
            }),
        ];
        for v in &variants {
            assert!(
                !(v.is_retryable() && v.is_terminal()),
                "retryable and terminal both true for {v:?}"
            );
        }
        assert!(PiiError::Timeout { after_ms: 1 }.is_retryable());
        assert!(!PiiError::Timeout { after_ms: 1 }.is_terminal());
        // The third class: neither retryable nor terminal (NotFound, Internal).
        // Pinned deliberately so a future change cannot silently make them
        // retryable (which would retry Internal forever).
        assert!(
            !PiiError::NotFound(String::new()).is_retryable()
                && !PiiError::NotFound(String::new()).is_terminal(),
            "NotFound must be neither retryable nor terminal"
        );
        assert!(
            !PiiError::Internal(String::new()).is_retryable()
                && !PiiError::Internal(String::new()).is_terminal(),
            "Internal must be neither retryable nor terminal"
        );
        for (v, name) in [
            (&PiiError::Denied(String::new()), "Denied"),
            (&PiiError::Invalid(String::new()), "Invalid"),
            (
                &PiiError::BudgetExhausted(String::from("Tokens")),
                "BudgetExhausted",
            ),
            (&PiiError::Cancelled, "Cancelled"),
        ] {
            assert!(v.is_terminal(), "{name} should be terminal");
        }
    }

    /// Spec 47 R2: the `StoreOp` display carries all three context fields.
    #[test]
    fn store_op_display_carries_context() {
        let err = PiiError::StoreOp {
            op: String::from("commit"),
            resource: String::from("heal_attempt"),
            detail: String::from("disk full"),
        };
        let s = err.to_string();
        assert!(s.contains("commit"), "display must contain op: {s}");
        assert!(
            s.contains("heal_attempt"),
            "display must contain resource: {s}"
        );
        assert!(s.contains("disk full"), "display must contain detail: {s}");
    }

    /// Spec 47 R2: `StoreOp` classification is pinned — like `Internal` and
    /// `NotFound`, it is NEITHER retryable nor terminal (a store failure may
    /// be transient or permanent; callers decide, so blanket-retrying would
    /// spin forever on a disk full).
    #[test]
    fn store_op_retry_terminal_classification() {
        let err = PiiError::StoreOp {
            op: String::from("commit"),
            resource: String::from("heal_attempt"),
            detail: String::from("disk full"),
        };
        assert!(
            !err.is_retryable() && !err.is_terminal(),
            "StoreOp must be neither retryable nor terminal (pinned)"
        );
    }

    fn provider_failure(status: Option<u16>, message: &str) -> ProviderFailure {
        ProviderFailure {
            status,
            origin: Origin::Provider,
            provider: String::from("ollama"),
            message: String::from(message),
        }
    }

    #[test]
    fn provider_error_is_retryable_iff_recovery_is_retry_same() {
        let err = PiiError::Provider(provider_failure(Some(500), ""));
        assert!(err.is_retryable());
        assert!(!err.is_terminal());
    }

    #[test]
    fn provider_error_next_provider_is_not_retryable_or_terminal() {
        let err = PiiError::Provider(provider_failure(Some(401), ""));
        assert!(!err.is_retryable());
        assert!(!err.is_terminal());
    }

    #[test]
    fn provider_error_compress_is_not_retryable_or_terminal() {
        let err = PiiError::Provider(provider_failure(None, "llamacpp: prompt too long"));
        assert!(!err.is_retryable());
        assert!(!err.is_terminal());
    }
}
