//! Bounded, in-process latency histograms (spec 115 R4).
//!
//! A [`Histogram`] is a fixed array of atomic counters over geometric bucket
//! bounds (ratio 1.1, whole milliseconds, strictly increasing), so memory is
//! constant however many runs are recorded and recording never locks.
//!
//! **Resolution — what a percentile means.** A reported percentile is the
//! UPPER bound of the bucket holding that rank, capped at the largest value
//! seen. It therefore never under-reports, and over-reports by at most 10%
//! (plus 1 ms at the low end, where bounds are 1, 2, 3, ...). **What it cannot
//! see**: durations are rounded UP to whole milliseconds (a 0.2 ms run is 1
//! ms); a duration beyond the last bound (about 24 minutes) lands in the last
//! bucket, so only `max_ms` is exact for such a run; and a percentile of fewer
//! than ~100 samples is just an order statistic, not a tail estimate.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Geometric growth between bucket bounds.
const RATIO: f64 = 1.1;
/// Number of buckets; the last bound is about 24 minutes.
const BUCKETS: usize = 150;

/// The one-line statement of resolution shown next to the numbers.
pub const LATENCY_RESOLUTION: &str = "percentiles are the upper bound of a 10%-wide bucket (never under-reported, at most 10% + 1 ms over), whole ms rounded up, durations past ~24 min saturate; max is exact";

fn bounds() -> &'static [u64; BUCKETS] {
    static BOUNDS: OnceLock<[u64; BUCKETS]> = OnceLock::new();
    BOUNDS.get_or_init(|| {
        let mut b = [0u64; BUCKETS];
        let mut prev = 0u64;
        for (i, slot) in b.iter_mut().enumerate() {
            let geo = RATIO.powi(i32::try_from(i).unwrap_or(i32::MAX)).ceil();
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let geo = geo as u64;
            prev = geo.max(prev + 1);
            *slot = prev;
        }
        b
    })
}

/// A fixed-size latency histogram.
pub struct Histogram {
    counts: [AtomicU64; BUCKETS],
    max_ms: AtomicU64,
}

/// Run-lifecycle outcomes the engine records, one histogram each.
pub const RUN_OUTCOMES: [&str; 4] = ["succeeded", "failed", "suspended", "refused"];

/// Run-lifecycle latency (accepted -> terminal) by outcome.
#[derive(Default)]
pub struct RunLatency {
    by_outcome: [Histogram; 4],
}

impl RunLatency {
    /// Records a run that ended as `outcome` (one of [`RUN_OUTCOMES`]); an
    /// unknown outcome is counted as `failed` rather than dropped.
    pub fn record(&self, outcome: &str, d: Duration) {
        let i = RUN_OUTCOMES.iter().position(|o| *o == outcome).unwrap_or(1);
        if let Some(h) = self.by_outcome.get(i) {
            h.record(d);
        }
    }

    /// Every outcome with its percentiles, in [`RUN_OUTCOMES`] order.
    #[must_use]
    pub fn snapshot(&self) -> Vec<(&'static str, Percentiles)> {
        RUN_OUTCOMES
            .iter()
            .zip(&self.by_outcome)
            .map(|(o, h)| (*o, h.snapshot()))
            .collect()
    }
}

/// A point-in-time read of a [`Histogram`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Percentiles {
    /// Samples recorded.
    pub count: u64,
    /// 50th percentile, ms.
    pub p50_ms: u64,
    /// 95th percentile, ms.
    pub p95_ms: u64,
    /// 99th percentile, ms.
    pub p99_ms: u64,
    /// Largest sample, ms (exact).
    pub max_ms: u64,
}

impl Default for Histogram {
    fn default() -> Self {
        Self {
            counts: std::array::from_fn(|_| AtomicU64::new(0)),
            max_ms: AtomicU64::new(0),
        }
    }
}

impl Histogram {
    /// Records one duration.
    pub fn record(&self, d: Duration) {
        let ms = u64::try_from(d.as_micros().div_ceil(1000))
            .unwrap_or(u64::MAX)
            .max(1);
        let idx = bounds().partition_point(|b| *b < ms).min(BUCKETS - 1);
        if let Some(c) = self.counts.get(idx) {
            c.fetch_add(1, Ordering::Relaxed);
        }
        self.max_ms.fetch_max(ms, Ordering::Relaxed);
    }

    /// Reads the percentiles. All zeros when nothing was recorded.
    #[must_use]
    pub fn snapshot(&self) -> Percentiles {
        let counts: Vec<u64> = self
            .counts
            .iter()
            .map(|c| c.load(Ordering::Relaxed))
            .collect();
        let count: u64 = counts.iter().sum();
        if count == 0 {
            return Percentiles::default();
        }
        let max_ms = self.max_ms.load(Ordering::Relaxed);
        let at = |q_percent: u64| {
            // Smallest rank r (1-based) with r >= q% of count.
            let rank = (count * q_percent).div_ceil(100).max(1);
            let mut seen = 0u64;
            for (i, c) in counts.iter().enumerate() {
                seen += c;
                if seen >= rank {
                    // The last bucket is open-ended: only the max is true there.
                    return if i == BUCKETS - 1 {
                        max_ms
                    } else {
                        bounds().get(i).map_or(max_ms, |b| (*b).min(max_ms))
                    };
                }
            }
            max_ms
        };
        Percentiles {
            count,
            p50_ms: at(50),
            p95_ms: at(95),
            p99_ms: at(99),
            max_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn bounds_are_strictly_increasing_and_reach_past_20_minutes() {
        let b = bounds();
        assert!(b.windows(2).all(|w| w[0] < w[1]));
        assert!(b[BUCKETS - 1] > 20 * 60 * 1000);
    }

    /// A percentile is within the stated resolution of the true order
    /// statistic: never below it, at most 10% + 1 ms above.
    #[test]
    fn percentiles_stay_within_the_stated_resolution() {
        let h = Histogram::default();
        for v in 1..=1000u64 {
            h.record(ms(v));
        }
        let s = h.snapshot();
        assert_eq!(s.count, 1000);
        for (got, truth) in [(s.p50_ms, 500u64), (s.p95_ms, 950), (s.p99_ms, 990)] {
            assert!(got >= truth, "under-reported: {got} < {truth}");
            assert!(
                got * 10 <= truth * 11 + 10,
                "over-reported past 10% + 1ms: {got} vs {truth}"
            );
        }
        assert_eq!(s.max_ms, 1000, "max is exact");
    }

    #[test]
    fn run_latency_keeps_outcomes_apart() {
        let r = RunLatency::default();
        r.record("succeeded", ms(10));
        r.record("failed", ms(500));
        r.record("nonsense", ms(500));
        let snap = r.snapshot();
        let get = |o: &str| snap.iter().find(|(n, _)| *n == o).unwrap().1;
        assert_eq!(get("succeeded").count, 1);
        assert_eq!(get("failed").count, 2, "unknown outcome counts as failed");
        assert!(get("succeeded").p50_ms < 20 && get("failed").p50_ms >= 500);
        assert_eq!(get("refused").count, 0);
    }

    #[test]
    fn sub_millisecond_rounds_up_and_an_empty_histogram_is_zero() {
        let h = Histogram::default();
        assert_eq!(h.snapshot(), Percentiles::default());
        h.record(Duration::from_micros(200));
        assert_eq!(h.snapshot().p50_ms, 1);
    }

    #[test]
    fn a_duration_past_the_last_bound_saturates_but_max_stays_exact() {
        let h = Histogram::default();
        h.record(Duration::from_hours(3));
        let s = h.snapshot();
        assert_eq!(s.count, 1);
        assert_eq!(s.max_ms, 3 * 3600 * 1000);
        assert_eq!(s.p99_ms, s.max_ms, "capped at the max, which is exact");
    }
}
