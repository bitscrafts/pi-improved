//! Kernel error type.

use thiserror::Error;

/// Errors surfaced by the Pii kernel ABI.
#[derive(Debug, Error)]
pub enum PiiError {
    /// An input or parameter was not valid.
    #[error("invalid: {0}")]
    Invalid(String),
    /// An operation was denied by policy or capability.
    #[error("denied: {0}")]
    Denied(String),
    /// An operation was cancelled before completion.
    #[error("cancelled")]
    Cancelled,
    /// A requested item was not found.
    #[error("not found: {0}")]
    NotFound(String),
    /// An unrecoverable internal fault.
    #[error("internal: {0}")]
    Internal(String),
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
}