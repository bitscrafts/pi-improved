//! The state directory: where the durable stores live (spec 351).
//!
//! The daemon, `pii backup`, and every CLI store default resolve it through [`state_dir`], so a default
//! command reads the store the daemon writes. The socket is not state: it stays in the runtime dir.

use std::path::PathBuf;

/// `$PII_STATE_DIR` when set and non-empty, else `$HOME/.local/state/pii`.
#[must_use]
pub fn state_dir() -> PathBuf {
    state_dir_from(&crate::env_var)
}

/// [`state_dir`] over an arbitrary variable lookup, so a caller that already holds a per-instance env seam
/// (the daemon's `build_with_env`, a test's scratch) resolves the SAME rule against its own view.
///
/// Measured 2026-11-03 (spec 351): the daemon called the process-global [`state_dir`] while its tests injected a
/// scratch `PII_STATE_DIR`, so three `channel_wiring` tests opened the operator's `channel.redb` and failed
/// together on `Database already open`.
#[must_use]
pub fn state_dir_from(env: &dyn Fn(&str) -> Option<String>) -> PathBuf {
    env("PII_STATE_DIR").filter(|s| !s.is_empty()).map_or_else(
        || {
            let home = env("HOME").unwrap_or_else(|| String::from("/root"));
            PathBuf::from(home).join(".local/state/pii")
        },
        PathBuf::from,
    )
}
