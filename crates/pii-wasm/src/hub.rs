//! The late-bound slot the daemon fills from `[wasm]`.

use std::sync::Arc;

use parking_lot::Mutex;
use pii_abi::PiiError;

use crate::WasmHost;

/// Holds the daemon's [`WasmHost`], if one is configured. Starts empty: the
/// plugin host is off unless configured.
#[derive(Default)]
pub struct WasmHub {
    slot: Mutex<Option<Arc<WasmHost>>>,
}

impl WasmHub {
    /// Installs the configured host (replacing any earlier one).
    pub fn set(&self, host: Arc<WasmHost>) {
        *self.slot.lock() = Some(host);
    }

    /// The configured host, if any.
    #[must_use]
    pub fn get(&self) -> Option<Arc<WasmHost>> {
        self.slot.lock().clone()
    }

    /// The configured host.
    ///
    /// # Errors
    /// [`PiiError::Denied`] when no `[wasm]` section configured a module dir.
    pub fn require(&self) -> Result<Arc<WasmHost>, PiiError> {
        self.get().ok_or_else(|| {
            PiiError::Denied(String::from(
                "wasm: not configured (the plugin host is off unless the daemon config has a [wasm] section)",
            ))
        })
    }
}
