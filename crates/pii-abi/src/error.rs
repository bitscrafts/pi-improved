//! Kernel error type.

use thiserror::Error;

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
}

impl PiiError {
    /// True when re-running the failed operation is worth trying.
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        matches!(self, Self::Timeout { .. })
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
}
