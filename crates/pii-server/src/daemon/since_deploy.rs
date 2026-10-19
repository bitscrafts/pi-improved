//! Run attribution (spec 188): the origin every run is stamped with, and the
//! `since_deploy` block of `sys.status`, read from run headers in the store.

use std::sync::OnceLock;

use std::collections::BTreeSet;

use pii_abi::{NodeId, PiiError, RunEvent, RunId, RunOrigin, RunStatus};
use pii_graph::spec::GraphSpec;
use pii_proto::sys::SinceDeploy;
use pii_store::Store;

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
