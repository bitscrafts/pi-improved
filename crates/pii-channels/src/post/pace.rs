//! Per-channel pacing from the `retry_after` a provider's refusal carries (spec 195).
//!
//! The pace is a state ("not before T"), never a sleep: the next `post` checks it and returns at once.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use pii_abi::PiiError;

use crate::retry_after_ms;

/// The instant before which each channel must not be dialled.
#[derive(Default)]
pub(super) struct Pacer {
    until: Mutex<BTreeMap<String, Instant>>,
}

impl Pacer {
    /// Milliseconds left in `channel`'s window, or `None` when it may be dialled now.
    pub(super) fn remaining_ms(&self, channel: &str) -> Option<u64> {
        let mut map = self.until.lock();
        let left = map
            .get(channel)
            .map(|t| t.saturating_duration_since(Instant::now()));
        match left {
            Some(d) if !d.is_zero() => Some(u64::try_from(d.as_millis()).unwrap_or(u64::MAX)),
            Some(_) => {
                map.remove(channel);
                None
            }
            None => None,
        }
    }

    /// Records a send's outcome: an error carrying `retry_after` starts a window; any other outcome
    /// (a success, an error without the token) clears it.
    pub(super) fn observe(&self, channel: &str, outcome: Result<(), &PiiError>) {
        let ms = outcome.err().and_then(retry_after_ms);
        let mut map = self.until.lock();
        match ms {
            Some(ms) => {
                map.insert(
                    channel.to_owned(),
                    Instant::now() + Duration::from_millis(ms),
                );
            }
            None => {
                map.remove(channel);
            }
        }
    }
}
