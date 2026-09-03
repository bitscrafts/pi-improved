//! Effect classification for parallel scheduling decisions.
//!
//! A bitflags set (pass-1 §2.5) so a node can declare a *combination* of
//! effects (e.g. `READS_FS | NET`) and so the set fails closed: the `Default`
//! is `WRITES_FS | MUTATES_EXTERNAL`, i.e. a node that does not declare its
//! effects is treated as serialised, never as parallel-safe.

use bitflags::bitflags;
use serde::{Deserialize, Serialize};

bitflags! {
    /// Classifies what external state a node touches.
    ///
    /// Parallel safety (pass-2 §1.2 / pass-3 §4.3): only effects that never
    /// mutate shared state may run concurrently.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct Effects: u16 {
        /// Touches nothing but its own inputs.
        const PURE = 1;
        /// Reads the filesystem; concurrent reads are safe.
        const READS_FS = 2;
        /// Writes the filesystem; exclusive access required.
        const WRITES_FS = 4;
        /// Reaches the network.
        const NET = 8;
        /// Spawns processes.
        const SPAWNS_PROC = 16;
        /// Mutates state outside the process.
        const MUTATES_EXTERNAL = 32;
        /// Long-running (streaming, background).
        const LONG_RUNNING = 64;
    }
}

impl Default for Effects {
    /// Fails closed: a node that does not declare its effects is treated as
    /// serialised (`WRITES_FS | MUTATES_EXTERNAL`), never parallel-safe.
    fn default() -> Self {
        Self::WRITES_FS | Self::MUTATES_EXTERNAL
    }
}

impl Effects {
    /// The effects that require exclusive (serialised) execution.
    const SERIALISED: Self = Self::WRITES_FS
        .union(Self::MUTATES_EXTERNAL)
        .union(Self::SPAWNS_PROC);

    /// True when two nodes with this effect may run in parallel.
    #[must_use]
    pub const fn is_parallel_safe(self) -> bool {
        !self.intersects(Self::SERIALISED)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::Effects;

    #[test]
    fn effects_parallel_safety() {
        assert!(Effects::PURE.is_parallel_safe());
        assert!(Effects::READS_FS.is_parallel_safe());
        assert!(!Effects::WRITES_FS.is_parallel_safe());
        assert!(!Effects::MUTATES_EXTERNAL.is_parallel_safe());
        assert!(!Effects::SPAWNS_PROC.is_parallel_safe());
        // A combination that reads the fs AND talks the network is still
        // parallel-safe (neither mutates shared state).
        assert!((Effects::READS_FS | Effects::NET).is_parallel_safe());
        // A node that writes the fs is never parallel-safe, even combined.
        assert!(!(Effects::READS_FS | Effects::WRITES_FS).is_parallel_safe());
    }

    #[test]
    fn effects_default_fails_closed() {
        // A node that declares no effects is serialised, not parallel-safe.
        assert!(!Effects::default().is_parallel_safe());
        assert!(Effects::default().intersects(Effects::WRITES_FS));
        assert!(Effects::default().intersects(Effects::MUTATES_EXTERNAL));
    }
}
