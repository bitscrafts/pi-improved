//! Thresholds and read bounds (spec 123 R2, R6). Every number here is a
//! stated, tested default; none is learned.

use serde::{Deserialize, Serialize};

/// Most run ids a candidate lists as evidence; the true count stays in `counts`.
pub const MAX_EVIDENCE_RUNS: usize = 50;

/// What an extraction scan reads and what it flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearnConfig {
    /// Distinct runs that must show the same failure/limit before it is a
    /// lesson. 3 is the smallest N where "repeated" is not "twice".
    pub min_runs: u32,
    /// A run is an outlier at `outlier_factor` times the graph's median.
    pub outlier_factor: u32,
    /// Runs of one graph needed before a median means anything.
    pub outlier_min_runs: u32,
    /// Duration outliers must also exceed this many ms (a 0 ms median makes
    /// every nonzero run an infinite-times outlier).
    pub outlier_min_ms: u64,
    /// Bytes-written outliers must also exceed this many bytes.
    pub outlier_min_bytes: u64,
    /// Most trajectory files one scan opens; the rest are counted, not read.
    pub max_files: usize,
    /// Largest single file read; a larger one is skipped and counted. Equals
    /// the trajectory sink's own per-run cap, so a capped run still fits.
    pub max_file_bytes: u64,
    /// Most bytes one scan reads in total.
    pub max_total_bytes: u64,
}

impl Default for LearnConfig {
    fn default() -> Self {
        Self {
            min_runs: 3,
            outlier_factor: 5,
            outlier_min_runs: 5,
            outlier_min_ms: 1_000,
            outlier_min_bytes: 4_096,
            max_files: 5_000,
            max_file_bytes: 16 * 1024 * 1024,
            max_total_bytes: 256 * 1024 * 1024,
        }
    }
}
