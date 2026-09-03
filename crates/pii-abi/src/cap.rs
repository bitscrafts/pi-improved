//! Coarse capability bits (pass-1 §2.4).
//!
//! Lives in `pii-abi` (not `pii-core`) so the capability surface is part of
//! the stable ABI: a WASM/worker node or plugin can reference `CapKind`
//! without depending on `pii-core` (opus review F7). `pii-core` re-exports it.

use crate::PiiError;
use bitflags::bitflags;

bitflags! {
    /// Coarse capability bits. Values must remain stable forever.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct CapKind: u32 {
        /// Read files within an allow path scope.
        const FS_READ = 1;
        /// Write files within an allow path scope.
        const FS_WRITE = 2;
        /// Reach the network.
        const NET = 4;
        /// Spawn/talk to processes.
        const PROC = 8;
        /// Touch hardware.
        const HW = 16;
        /// Access secrets.
        const SECRETS = 32;
        /// Write into the agent's memory.
        const MEMORY_WRITE = 64;
        /// Spawn subprocesses.
        const SPAWN = 128;
    }
}

impl CapKind {
    /// Parses a `snake_case` capability name.
    ///
    /// # Errors
    /// Returns [`PiiError::Invalid`] for an unknown name.
    pub fn parse_kind(s: &str) -> Result<Self, PiiError> {
        match s {
            "fs_read" => Ok(Self::FS_READ),
            "fs_write" => Ok(Self::FS_WRITE),
            "net" => Ok(Self::NET),
            "proc" => Ok(Self::PROC),
            "hw" => Ok(Self::HW),
            "secrets" => Ok(Self::SECRETS),
            "memory_write" => Ok(Self::MEMORY_WRITE),
            "spawn" => Ok(Self::SPAWN),
            other => Err(PiiError::Invalid(format!(
                "unknown capability kind '{other}'"
            ))),
        }
    }
}
