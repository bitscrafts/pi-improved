//! Host configuration and per-call options.

use std::path::PathBuf;
use std::time::Duration;

/// Default module entry point.
pub const DEFAULT_ENTRY: &str = "run";
/// Default (and ceiling) wall-clock budget for one call.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(1);
/// Default per-instance memory ceiling (16 MiB).
pub const DEFAULT_MAX_MEMORY: usize = 16 * 1024 * 1024;
/// Default fuel ceiling when a node enables fuel without a number.
pub const DEFAULT_FUEL: u64 = 50_000_000;
/// Largest `.wasm` file the host will read and compile (compilation is not
/// interruptible, so the input is bounded instead).
pub const MAX_MODULE_BYTES: u64 = 8 * 1024 * 1024;

/// The host's operator-set limits (`[wasm]` in the daemon config).
#[derive(Debug, Clone)]
pub struct WasmConfig {
    /// The only directory modules may be loaded from.
    pub dir: PathBuf,
    /// Wall-clock budget per call; a node may only lower it.
    pub timeout: Duration,
    /// Memory ceiling per instance, in bytes.
    pub max_memory: usize,
    /// The fuel a node gets when it enables fuel without a number; also the
    /// most a node may ask for.
    pub fuel: u64,
}

impl WasmConfig {
    /// A config over `dir` with the documented defaults.
    #[must_use]
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            timeout: DEFAULT_TIMEOUT,
            max_memory: DEFAULT_MAX_MEMORY,
            fuel: DEFAULT_FUEL,
        }
    }
}

/// Fuel metering for one call.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Fuel {
    /// No metering (the epoch deadline still applies).
    #[default]
    Off,
    /// Meter against the configured ceiling.
    Default,
    /// Meter against this many units (capped at the configured ceiling).
    Limit(u64),
}

/// Per-call options a `wasm` node supplies.
#[derive(Debug, Clone, Default)]
pub struct CallOpts {
    /// Export to call; [`DEFAULT_ENTRY`] when `None`.
    pub entry: Option<String>,
    /// Lower wall-clock budget than the configured one.
    pub timeout: Option<Duration>,
    /// Fuel metering.
    pub fuel: Fuel,
}
