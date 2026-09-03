//! Environment configuration access.
//!
//! The workspace clippy config disallows `std::env::var` (non-UTF-8 and
//! non-UTF-16 lossiness); every env read goes through [`env_var`], which uses
//! `var_os` and requires valid UTF-8. Phase 2 spec F1.8/F2.8/F3.10 read
//! `PII_MODEL`, `PII_RUNTIME_DIR`, `OLLAMA_HOST`, and friends through here.

/// Reads an environment variable, returning `None` when unset or not UTF-8.
#[must_use]
pub fn env_var(name: &str) -> Option<String> {
    std::env::var_os(name).and_then(|v| v.into_string().ok())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    #[test]
    fn env_var_missing_is_none() {
        assert!(super::env_var("PII_DEFINITELY_UNSET_VAR_XYZ").is_none());
    }

    #[test]
    fn env_var_path_is_some() {
        // PATH is set in every environment this suite runs in.
        assert!(super::env_var("PATH").is_some());
    }
}
