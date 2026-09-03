//! Injectable clock (F1.3): the ONLY `SystemTime::now` call site in the
//! workspace lives in [`SystemClock`], so every wall-clock read added by
//! Phase 3 (timer wheel, cron, grant expiry, decay) is table-testable
//! through [`FixedClock`].

use std::sync::atomic::{AtomicU64, Ordering};

/// A source of wall-clock milliseconds since the Unix epoch.
pub trait Clock: Send + Sync {
    /// Current time in milliseconds since the epoch.
    fn now_ms(&self) -> u64;
}

/// The real clock. The single sanctioned `SystemTime::now` site (F1.3).
pub struct SystemClock;

impl Clock for SystemClock {
    #[allow(clippy::disallowed_methods)] // reason: SystemClock is the ONLY
    // SystemTime::now call site in the workspace (spec F1.3); all other
    // reads go through the `Clock` trait so tests can freeze time.
    fn now_ms(&self) -> u64 {
        u64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis()),
        )
        .unwrap_or(u64::MAX)
    }
}

/// A controllable clock for tests: starts at 0, advances only when told.
#[derive(Debug, Default)]
pub struct FixedClock {
    ms: AtomicU64,
}

impl FixedClock {
    /// A clock at epoch 0.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Advances the frozen time by `ms` milliseconds.
    pub fn advance(&self, ms: u64) {
        self.ms.fetch_add(ms, Ordering::Relaxed);
    }
}

impl Clock for FixedClock {
    fn now_ms(&self) -> u64 {
        self.ms.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use crate::clock::{Clock, FixedClock, SystemClock};

    #[test]
    fn fixed_clock_advances() {
        let c = FixedClock::new();
        assert_eq!(c.now_ms(), 0);
        c.advance(1500);
        assert_eq!(c.now_ms(), 1500);
    }

    #[test]
    fn system_clock_is_past_2020() {
        assert!(SystemClock.now_ms() > 1_577_836_800_000);
    }
}
