//! Effect classification for parallel scheduling decisions.

use serde::{Deserialize, Serialize};

/// Classifies what external state a node touches.
///
/// Parallel safety (pass-2 §1.2 / pass-3 §4.3): only effects that never
/// mutate shared state may run concurrently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum Effects {
    /// Touches nothing but its own inputs.
    Pure,
    /// Reads the filesystem; concurrent reads are safe.
    ReadsFs,
    /// Writes the filesystem; exclusive access required.
    WritesFs,
    /// Mutates state outside the process.
    MutatesExternal,
}

impl Effects {
    /// True when two nodes with this effect may run in parallel.
    #[must_use]
    pub const fn is_parallel_safe(self) -> bool {
        matches!(self, Self::Pure | Self::ReadsFs)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::Effects;

    #[test]
    fn effects_parallel_safety() {
        assert!(Effects::Pure.is_parallel_safe());
        assert!(Effects::ReadsFs.is_parallel_safe());
        assert!(!Effects::WritesFs.is_parallel_safe());
        assert!(!Effects::MutatesExternal.is_parallel_safe());
    }
}