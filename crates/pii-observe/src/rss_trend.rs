//! RSS growth per run, over a bounded window of recent polls (spec 116 R2).
//!
//! A leak nobody can see is how F2 (spec 110) went unnoticed: `pii_process_rss_kb`
//! and `pii_runs` exist, but a slope needs two points and a division. This keeps
//! up to [`TREND_POINTS`] checkpoints, at least [`TREND_STEP_RUNS`] runs apart,
//! and reports the growth between the oldest and now as bytes per run.
//!
//! **What it cannot see**: it is an average over the window, not an
//! instantaneous slope, so a leak that starts late is diluted for a while; it is
//! `0` until [`TREND_MIN_SPAN_RUNS`] runs separate the window ends, and `0`
//! when RSS shrank. The first ~20 000 runs of a fresh store are warm-up (the
//! page caches fill) and read high; see the runbook for what to expect after.

use std::collections::VecDeque;

/// A new checkpoint is taken once this many runs passed since the last.
pub const TREND_STEP_RUNS: u64 = 1_000;
/// Checkpoints kept (so the window is up to about 32 000 runs).
pub const TREND_POINTS: usize = 32;
/// The window must span at least this many runs before a number is reported.
pub const TREND_MIN_SPAN_RUNS: u64 = 4_000;

/// Bounded checkpoints of `(runs, rss_kb)`.
// reason: pii-observe is the dependency floor (serde only); it cannot take
// parking_lot, and a poisoned lock just drops a checkpoint.
#[allow(clippy::disallowed_types)]
#[derive(Default)]
pub struct RssTrend {
    points: std::sync::Mutex<VecDeque<(u64, u64)>>,
}

impl RssTrend {
    /// A trend with no checkpoints.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Notes the current `(runs, rss_kb)`; kept only when `runs` advanced by
    /// [`TREND_STEP_RUNS`] since the last checkpoint.
    pub fn note(&self, runs: u64, rss_kb: u64) {
        let Ok(mut p) = self.points.lock() else {
            return;
        };
        if p.back().is_some_and(|(r, _)| runs < r + TREND_STEP_RUNS) {
            return;
        }
        p.push_back((runs, rss_kb));
        while p.len() > TREND_POINTS {
            p.pop_front();
        }
    }

    /// Bytes of RSS growth per run between the oldest checkpoint and
    /// `(runs, rss_kb)`; `0` when the window is too short or RSS did not grow.
    #[must_use]
    pub fn bytes_per_run(&self, runs: u64, rss_kb: u64) -> u64 {
        let Ok(p) = self.points.lock() else {
            return 0;
        };
        let Some(&(r0, kb0)) = p.front() else {
            return 0;
        };
        let span = runs.saturating_sub(r0);
        if span < TREND_MIN_SPAN_RUNS {
            return 0;
        }
        rss_kb.saturating_sub(kb0).saturating_mul(1024) / span
    }

    /// How many checkpoints are held (bounded by [`TREND_POINTS`]).
    #[must_use]
    pub fn len(&self) -> usize {
        self.points.lock().map_or(0, |p| p.len())
    }

    /// Whether no checkpoint is held.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feeds `runs_total` runs polled every 500 runs, RSS growing `bytes_per_run`.
    fn fed(runs_total: u64, base_kb: u64, bytes_per_run: u64) -> (RssTrend, u64, u64) {
        let t = RssTrend::new();
        let (mut runs, mut kb) = (0, base_kb);
        while runs < runs_total {
            t.note(runs, kb);
            runs += 500;
            kb = base_kb + runs * bytes_per_run / 1024;
        }
        (t, runs, kb)
    }

    #[test]
    fn a_steady_leak_reads_as_its_bytes_per_run() {
        let (t, runs, kb) = fed(20_000, 40_000, 150);
        let got = t.bytes_per_run(runs, kb);
        assert!((140..=160).contains(&got), "{got}");
    }

    #[test]
    fn flat_or_shrinking_rss_reads_zero() {
        let (t, runs, kb) = fed(20_000, 40_000, 0);
        assert_eq!(t.bytes_per_run(runs, kb), 0);
        assert_eq!(t.bytes_per_run(runs, 1), 0, "shrinkage is not growth");
    }

    #[test]
    fn a_short_window_reads_zero_rather_than_a_misleading_number() {
        let t = RssTrend::new();
        t.note(0, 10_000);
        assert_eq!(t.bytes_per_run(TREND_MIN_SPAN_RUNS - 1, 90_000), 0);
        assert!(t.bytes_per_run(TREND_MIN_SPAN_RUNS, 90_000) > 0);
        assert_eq!(
            RssTrend::new().bytes_per_run(50_000, 90_000),
            0,
            "no checkpoint"
        );
    }

    #[test]
    fn the_trend_itself_is_bounded() {
        let t = RssTrend::new();
        for i in 0..10_000u64 {
            t.note(i * TREND_STEP_RUNS, 40_000);
        }
        assert_eq!(t.len(), TREND_POINTS);
    }
}
