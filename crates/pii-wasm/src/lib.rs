//! `pii-wasm` (spec 122): the WASM plugin host — third-party graph nodes
//! without rebuilding `pii`.
//!
//! A plugin is **untrusted code running inside the daemon**, so the host is
//! built around what a module cannot do:
//!
//! * **Wall-clock budget** — epoch interruption; an infinite loop is
//!   interrupted and the call fails as [`pii_abi::PiiError::Timeout`].
//! * **Fuel** — opt-in per call, with a default ceiling when enabled.
//! * **Memory** — a per-instance cap; growth past it traps the call.
//! * **No ambient authority** — the linker is empty. A module that imports
//!   anything (WASI `path_open`, `fd_write`, `environ_get`, a socket, a host
//!   function) is refused before it runs. The host exposes **no** WASI.
//! * **Directory scope** — a module loads only from the configured `dir`.
//!
//! [`WasmHub`] starts empty: the host is off unless the daemon config has a
//! `[wasm]` section, and a `wasm` node then fails loudly with the reason.
//!
//! This is a boundary for the *module*, not for the host: a bug in wasmtime
//! is a bug in `pii`. See `_docs/02-guides/wasm-plugins.md`.

#![forbid(unsafe_code)]

mod config;
mod host;
mod hub;
mod state;

pub use config::{
    CallOpts, DEFAULT_ENTRY, DEFAULT_FUEL, DEFAULT_MAX_MEMORY, DEFAULT_TIMEOUT, Fuel,
    MAX_MODULE_BYTES, WasmConfig,
};
pub use host::{WASMTIME_VERSION, WasmHost};
pub use hub::WasmHub;
pub use state::WasmCounts;
