//! Aggregate retention for trajectory files (spec 115 R2).
//!
//! The per-run caps bound one file; this bounds the directory. The index is
//! built from one scan of the directory the first time a file is created, then
//! kept current from the sink's own writes, so enforcing the policy never
//! rescans a directory of tens of thousands of files. Pruning removes the
//! OLDEST finished trajectories first and never touches a run that is still
//! open; when only open runs remain and the budget is still exceeded, that is
//! reported (once per episode) instead of dropping anything.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use crate::{Event, Level, Observer, names};

/// Aggregate bounds on the trajectory directory. `None` = unbounded on that axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Retention {
    /// Maximum total bytes across all trajectory files.
    pub max_bytes: Option<u64>,
    /// Maximum number of trajectory files.
    pub max_files: Option<usize>,
}

/// Pruning stops at this percentage of a bound, so a steady stream of runs
/// prunes in batches (one event per batch) rather than once per run.
const LOW_WATER_PERCENT: u64 = 90;

/// The in-memory view of the trajectory directory.
// reason: `pub` trips rustc's unreachable_pub, `pub(crate)` trips clippy's
// redundant_pub_crate; the type is crate-internal.
#[allow(clippy::redundant_pub_crate)]
#[derive(Default)]
pub(crate) struct Index {
    scanned: bool,
    next: u64,
    order: BTreeMap<u64, String>,
    info: HashMap<String, (u64, u64)>,
    total: u64,
    over_reported: bool,
}

impl Index {
    /// Scans `dir` once, oldest file first. A missing directory is empty.
    pub(crate) fn ensure_scanned(&mut self, dir: &Path) {
        if self.scanned {
            return;
        }
        self.scanned = true;
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        let mut found: Vec<(std::time::SystemTime, String, u64)> = rd
            .filter_map(Result::ok)
            .filter_map(|e| {
                let name = e.file_name().into_string().ok()?;
                let md = e.metadata().ok()?;
                (md.is_file()
                    && Path::new(&name)
                        .extension()
                        .is_some_and(|x| x.eq_ignore_ascii_case("jsonl")))
                .then(|| {
                    (
                        md.modified().unwrap_or(std::time::UNIX_EPOCH),
                        name,
                        md.len(),
                    )
                })
            })
            .collect();
        found.sort();
        for (_, name, bytes) in found {
            self.add(name, bytes);
        }
    }

    fn add(&mut self, name: String, bytes: u64) {
        let seq = self.next;
        self.next += 1;
        self.total += bytes;
        self.order.insert(seq, name.clone());
        self.info.insert(name, (seq, bytes));
    }

    /// Records that `name` exists (idempotent: a reopened file keeps its age).
    pub(crate) fn created(&mut self, name: &str, bytes: u64) {
        if !self.info.contains_key(name) {
            self.add(name.to_owned(), bytes);
        }
    }

    /// Records `n` bytes appended to `name`.
    pub(crate) fn wrote(&mut self, name: &str, n: u64) {
        if let Some((_, bytes)) = self.info.get_mut(name) {
            *bytes += n;
            self.total += n;
        }
    }

    fn over(&self, r: &Retention) -> bool {
        r.max_bytes.is_some_and(|m| self.total > m)
            || r.max_files.is_some_and(|m| self.info.len() > m)
    }

    fn above_low_water(&self, r: &Retention) -> bool {
        let low = |m: u64| m / 100 * LOW_WATER_PERCENT + m % 100 * LOW_WATER_PERCENT / 100;
        r.max_bytes.is_some_and(|m| self.total > low(m))
            || r.max_files
                .is_some_and(|m| self.info.len() as u64 > low(m as u64))
    }

    /// Prunes oldest-first (skipping `open` runs' files) while over policy.
    pub(crate) fn enforce(
        &mut self,
        dir: &Path,
        r: &Retention,
        open: &HashSet<String>,
        reporter: &dyn Observer,
    ) {
        if !self.over(r) {
            self.over_reported = false;
            return;
        }
        let (mut files, mut bytes) = (0u64, 0u64);
        let candidates: Vec<(u64, String)> = self
            .order
            .iter()
            .filter(|(_, n)| !open.contains(*n))
            .map(|(s, n)| (*s, n.clone()))
            .collect();
        for (seq, name) in candidates {
            if !self.above_low_water(r) {
                break;
            }
            match std::fs::remove_file(dir.join(&name)) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    reporter.event(
                        &Event::new("observe", Level::Error, names::TRAJECTORY_WRITE_FAILED)
                            .field("file", name.as_str())
                            .field("error", format!("prune: {e}")),
                    );
                    continue;
                }
            }
            self.order.remove(&seq);
            if let Some((_, b)) = self.info.remove(&name) {
                self.total -= b;
                bytes += b;
            }
            files += 1;
        }
        if files > 0 {
            reporter.event(
                &Event::new("observe", Level::Warn, names::TRAJECTORY_PRUNED)
                    .field("files", files)
                    .field("bytes", bytes)
                    .field("remaining_files", self.info.len() as u64)
                    .field("remaining_bytes", self.total),
            );
        }
        if self.over(r) {
            if !self.over_reported {
                self.over_reported = true;
                reporter.event(
                    &Event::new("observe", Level::Error, names::TRAJECTORY_OVER_BUDGET)
                        .field("files", self.info.len() as u64)
                        .field("bytes", self.total)
                        .field("open_runs", open.len() as u64),
                );
            }
        } else {
            self.over_reported = false;
        }
    }
}
