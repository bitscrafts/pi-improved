//! Counters and the per-instance resource limiter.

// reason: `pub(crate)` is the widest `Inner`/`Limits` need (used only by the
// sibling `host` module); clippy pedantic `redundant_pub_crate` conflicts with
// rustc `unreachable_pub` (deny) here, so it is allowed narrowly.
#![allow(clippy::redundant_pub_crate)]

use wasmtime::ResourceLimiter;

/// Most table elements an instance may grow to.
const MAX_TABLE_ELEMENTS: usize = 10_000;

/// Call counts by outcome since the host was built (spec 122 R5).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WasmCounts {
    /// Calls that returned a valid JSON value.
    pub ok: u64,
    /// Calls that trapped.
    pub trap: u64,
    /// Calls interrupted by the epoch deadline.
    pub timeout: u64,
    /// Calls that ran out of fuel.
    pub fuel: u64,
    /// Calls refused memory beyond the cap.
    pub memory: u64,
    /// Calls that returned something that was not valid JSON.
    pub invalid: u64,
    /// Modules or calls refused before running.
    pub refusals: u64,
}

#[derive(Default)]
pub(crate) struct Inner {
    pub(crate) counts: WasmCounts,
    pub(crate) last_error: Option<String>,
}

/// The per-instance resource limits, enforced by the runtime on every growth.
pub(crate) struct Limits {
    pub(crate) max_memory: usize,
    pub(crate) denied: bool,
}

impl ResourceLimiter for Limits {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        if desired > self.max_memory {
            self.denied = true;
            return Err(wasmtime::Error::msg(format!(
                "memory limit exceeded ({desired} bytes wanted, {} allowed)",
                self.max_memory
            )));
        }
        Ok(true)
    }

    fn table_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        Ok(desired <= MAX_TABLE_ELEMENTS)
    }

    fn instances(&self) -> usize {
        1
    }

    fn tables(&self) -> usize {
        4
    }

    fn memories(&self) -> usize {
        1
    }
}
