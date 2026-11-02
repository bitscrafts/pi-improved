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
        /// Read from the agent's memory.
        const MEMORY_READ = 256;
        /// Post a message into a configured channel (spec 128).
        const POST = 512;
        /// Read the house's MQTT devices: list them and subscribe (spec 336).
        const MQTT_READ = 1024;
        /// Command a house's MQTT device through its allowlist (spec 336).
        const MQTT_WRITE = 2048;
    }
}

impl CapKind {
    /// The `snake_case` name of each capability set here, in declaration order (spec 215 refusals name them).
    #[must_use]
    pub fn names(self) -> Vec<&'static str> {
        const ALL: [(CapKind, &str); 12] = [
            (CapKind::FS_READ, "fs_read"),
            (CapKind::FS_WRITE, "fs_write"),
            (CapKind::NET, "net"),
            (CapKind::PROC, "proc"),
            (CapKind::HW, "hw"),
            (CapKind::SECRETS, "secrets"),
            (CapKind::MEMORY_WRITE, "memory_write"),
            (CapKind::SPAWN, "spawn"),
            (CapKind::MEMORY_READ, "memory_read"),
            (CapKind::POST, "post"),
            (CapKind::MQTT_READ, "mqtt_read"),
            (CapKind::MQTT_WRITE, "mqtt_write"),
        ];
        ALL.iter()
            .filter(|(kind, _)| self.contains(*kind))
            .map(|(_, name)| *name)
            .collect()
    }

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
            "memory_read" => Ok(Self::MEMORY_READ),
            "post" => Ok(Self::POST),
            "mqtt_read" => Ok(Self::MQTT_READ),
            "mqtt_write" => Ok(Self::MQTT_WRITE),
            other => Err(PiiError::Invalid(format!(
                "unknown capability kind '{other}'"
            ))),
        }
    }
}
