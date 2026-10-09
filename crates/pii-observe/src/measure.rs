//! Measures: numeric observations.

use serde::{Deserialize, Serialize};

/// A numeric observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Measure {
    /// A monotonically increasing count.
    Counter(u64),
    /// A value that can go up and down.
    Gauge(i64),
    /// A latency summary in microseconds.
    Histogram {
        /// Number of samples.
        count: u64,
        /// Sum of samples, microseconds.
        sum_us: u64,
        /// Largest sample, microseconds.
        max_us: u64,
    },
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn measure_serde_round_trip() {
        for m in [
            Measure::Counter(5),
            Measure::Gauge(-3),
            Measure::Histogram {
                count: 2,
                sum_us: 30,
                max_us: 20,
            },
        ] {
            let json = serde_json::to_string(&m).unwrap();
            assert_eq!(serde_json::from_str::<Measure>(&json).unwrap(), m);
        }
        assert_eq!(
            serde_json::to_string(&Measure::Gauge(-3)).unwrap(),
            r#"{"Gauge":-3}"#
        );
    }
}
