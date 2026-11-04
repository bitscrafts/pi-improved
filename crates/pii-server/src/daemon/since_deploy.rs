//! Run attribution (spec 188): the origin every run is stamped with, and the
//! `since_deploy` block of `sys.status`, read from run headers in the store.

use std::sync::OnceLock;

use std::collections::{BTreeMap, BTreeSet};

use pii_abi::{NodeId, PiiError, RunEvent, RunId, RunOrigin, RunStatus};
use pii_graph::spec::GraphSpec;
use pii_proto::sys::{RunsByStatus, SinceDeploy};
use pii_store::Store;
use serde::{Deserialize, Serialize};

use super::identity::REVISION;

/// The running process's origin: the build revision and the wall clock at
/// first call. Built once per process, so every run of one process shares it.
#[must_use]
pub fn process_origin() -> RunOrigin {
    static ORIGIN: OnceLock<RunOrigin> = OnceLock::new();
    ORIGIN
        .get_or_init(|| {
            use pii_core::Clock as _;
            RunOrigin {
                revision: REVISION.to_owned(),
                process_start_ms: pii_core::SystemClock.now_ms(),
            }
        })
        .clone()
}

/// Counts the runs in `store` whose header names `revision`, with their starts.
///
/// One pass over the run table also yields the distinct process starts and the
/// hard deaths among them. A header with no revision is `unattributed` and never
/// counts as any deploy's.
/// # Errors
/// Returns [`PiiError`] when the run table cannot be read.
pub fn since_deploy(store: &Store, revision: &str) -> Result<SinceDeploy, PiiError> {
    let mut out = SinceDeploy {
        revision: revision.to_owned(),
        ..SinceDeploy::default()
    };
    let mut ours: Vec<(RunId, String)> = Vec::new();
    let mut starts: BTreeSet<u64> = BTreeSet::new();
    let mut running_starts: Vec<u64> = Vec::new();
    store.for_each_run_header(|h| {
        if h.revision.is_empty() {
            out.unattributed += 1;
        } else if h.revision == revision {
            out.runs += 1;
            starts.insert(h.process_start_ms);
            if h.status == RunStatus::Running {
                running_starts.push(h.process_start_ms);
            }
            ours.push((h.run.clone(), h.graph.clone()));
            let by = &mut out.runs_by_status;
            match h.status {
                RunStatus::Running => by.running += 1,
                RunStatus::Suspended => by.suspended += 1,
                RunStatus::Succeeded => by.succeeded += 1,
                RunStatus::Failed => by.failed += 1,
                RunStatus::Refused => by.refused += 1,
            }
        }
    })?;
    out.armed = out.runs_by_status.suspended;
    out.starts = starts.len() as u64;
    if let Some(newest) = starts.last() {
        out.hard_deaths = running_starts.iter().filter(|s| *s < newest).count() as u64;
    }
    for (run, graph) in &ours {
        match run_posts(store, run, graph) {
            Some((posted, refused)) => {
                out.posted += posted;
                out.post_refused += refused;
            }
            None => out.unreadable += 1,
        }
    }
    Ok(out)
}

/// `(posted, refused)` for one run, from its stored graph body and journal;
/// `None` when either cannot be read. A duplicate post counts as neither.
fn run_posts(store: &Store, run: &RunId, graph: &str) -> Option<(u64, u64)> {
    let body = store.get_graph(graph).ok()??;
    let spec = GraphSpec::from_json(&body).ok()?;
    let post_nodes: BTreeSet<&NodeId> = spec
        .nodes
        .iter()
        .filter(|n| n.kind == "post")
        .map(|n| &n.id)
        .collect();
    if post_nodes.is_empty() {
        return Some((0, 0));
    }
    let (mut posted, mut refused) = (0, 0);
    for (_, ev) in store.read_journal(run).ok()? {
        match ev {
            RunEvent::NodeFinished { node, value, .. }
                if post_nodes.contains(&node)
                    && value.get("duplicate").and_then(serde_json::Value::as_bool)
                        != Some(true) =>
            {
                posted += 1;
            }
            RunEvent::NodeFailed { node, .. } if post_nodes.contains(&node) => refused += 1,
            _ => {}
        }
    }
    Some((posted, refused))
}

/// Spec 370: one revision's share of the `since_deploy` block. The health record keeps one per revision, so the block
/// is read rather than replayed; its fields mean what [`since_deploy`] reports.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevisionAgg {
    /// Runs whose header names the revision.
    pub runs: u64,
    /// `runs` split by status.
    pub by_status: RunsByStatus,
    /// Runs per process start that wrote the revision.
    pub starts: BTreeMap<u64, u64>,
    /// Runs still running, per process start.
    pub running_starts: BTreeMap<u64, u64>,
    /// Posts the runs made.
    pub posted: u64,
    /// Posts the runs attempted that were refused.
    pub post_refused: u64,
    /// Runs whose posts could not be counted.
    pub unreadable: u64,
}

impl RevisionAgg {
    /// What one run of the revision adds. `posts` is `(posted, refused)`, or `None` when the run's posts are unreadable.
    #[must_use]
    pub fn of_run(status: RunStatus, process_start_ms: u64, posts: Option<(u64, u64)>) -> Self {
        let mut agg = Self {
            runs: 1,
            ..Self::default()
        };
        match status {
            RunStatus::Running => {
                agg.by_status.running = 1;
                agg.running_starts.insert(process_start_ms, 1);
            }
            RunStatus::Suspended => agg.by_status.suspended = 1,
            RunStatus::Succeeded => agg.by_status.succeeded = 1,
            RunStatus::Failed => agg.by_status.failed = 1,
            RunStatus::Refused => agg.by_status.refused = 1,
        }
        agg.starts.insert(process_start_ms, 1);
        match posts {
            Some((posted, refused)) => {
                agg.posted = posted;
                agg.post_refused = refused;
            }
            None => agg.unreadable = 1,
        }
        agg
    }

    /// Adds `c` to this share, or takes it away when `add` is false.
    pub(super) fn combine(&mut self, c: &Self, add: bool) {
        bump(&mut self.runs, c.runs, add);
        bump(&mut self.by_status.running, c.by_status.running, add);
        bump(&mut self.by_status.suspended, c.by_status.suspended, add);
        bump(&mut self.by_status.succeeded, c.by_status.succeeded, add);
        bump(&mut self.by_status.failed, c.by_status.failed, add);
        bump(&mut self.by_status.refused, c.by_status.refused, add);
        bump(&mut self.posted, c.posted, add);
        bump(&mut self.post_refused, c.post_refused, add);
        bump(&mut self.unreadable, c.unreadable, add);
        merge_counts(&mut self.starts, &c.starts, add);
        merge_counts(&mut self.running_starts, &c.running_starts, add);
    }

    /// The `since_deploy` block for `revision`, given the store-wide count of unattributed runs.
    #[must_use]
    pub fn to_deploy(&self, revision: &str, unattributed: u64) -> SinceDeploy {
        let hard_deaths = self.starts.keys().next_back().map_or(0, |newest| {
            self.running_starts
                .range(..*newest)
                .map(|(_, n)| *n)
                .sum::<u64>()
        });
        SinceDeploy {
            revision: revision.to_owned(),
            runs: self.runs,
            runs_by_status: self.by_status.clone(),
            armed: self.by_status.suspended,
            unattributed,
            posted: self.posted,
            post_refused: self.post_refused,
            unreadable: self.unreadable,
            starts: self.starts.len() as u64,
            hard_deaths,
        }
    }
}

/// Adds `n` to `field`, or takes it away when `add` is false.
pub(super) fn bump(field: &mut u64, n: u64, add: bool) {
    if add {
        *field += n;
    } else {
        *field = field.saturating_sub(n);
    }
}

/// Merges per-key counts into `map`, dropping a key whose count reaches zero.
pub(super) fn merge_counts(map: &mut BTreeMap<u64, u64>, src: &BTreeMap<u64, u64>, add: bool) {
    for (key, n) in src {
        let left = {
            let slot = map.entry(*key).or_insert(0);
            bump(slot, *n, add);
            *slot
        };
        if left == 0 {
            map.remove(key);
        }
    }
}
