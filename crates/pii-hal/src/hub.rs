//! The late-bound slot the daemon fills from `[hal]`.

use std::sync::Arc;

use parking_lot::Mutex;
use pii_abi::PiiError;

use crate::Hal;

/// Holds the daemon's [`Hal`], if one is configured. Starts empty: the HAL is
/// off unless configured, and a graph that asks for it then fails loudly.
#[derive(Default)]
pub struct HalHub {
    slot: Mutex<Option<Arc<Hal>>>,
}

impl HalHub {
    /// Installs the configured HAL (replacing any earlier one).
    pub fn set(&self, hal: Arc<Hal>) {
        *self.slot.lock() = Some(hal);
    }

    /// The configured HAL, if any.
    #[must_use]
    pub fn get(&self) -> Option<Arc<Hal>> {
        self.slot.lock().clone()
    }

    /// The configured HAL.
    ///
    /// # Errors
    /// [`PiiError::Denied`] when no `[hal]` section configured a backend.
    pub fn require(&self) -> Result<Arc<Hal>, PiiError> {
        self.get().ok_or_else(|| {
            PiiError::Denied(String::from(
                "hal: not configured (the HAL is off unless the daemon config has a [hal] section)",
            ))
        })
    }
}
