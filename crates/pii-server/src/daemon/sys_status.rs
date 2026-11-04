//! The `SysStatus` record health reads instead of replaying every journal (spec 370).
//!
//! Extracted from `identity.rs`, which was over its line budget once the record existed.

use std::collections::{BTreeMap, BTreeSet};

use pii_abi::{NodeId, PiiError, RunEvent, RunHeader, RunStatus};
use pii_proto::sys::SinceDeploy;
use serde::{Deserialize, Serialize};

use pii_graph::agent_calls::Totals;

use crate::daemon::since_deploy::{RevisionAgg, bump, merge_counts};

const TOTALS_KEY: &str = "totals";
use pii_graph::spec::GraphSpec;
use pii_store::{
    Store, SysTable, WriteTransaction, for_each_run_journal, from_bytes, runs_up_to, sys_clear,
    sys_get, sys_put, to_bytes,
};

/// What `sys.status` reports from the journals (spec 370), kept current by every run write.
///
/// Health reads this one record instead of replaying each journal. Its numbers are the same as the
/// replay's (`agent_calls::store_totals` and `since_deploy::since_deploy`), and a rebuild from the
/// journals is the proof.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SysStatus {
    /// Agent calls ever spawned.
    dispatches_total: u64,
    /// Agent calls refused before any spawn (`worker.start.refused` observations).
    refusals_total: u64,
    /// Agent calls whose criteria did not all pass.
    criteria_failures_total: u64,
    /// Spawns with no exit in a run that reached a terminal state.
    orphans_total: u64,
    /// Spawns with no exit in a run that has not.
    agents_in_flight: u64,
    /// The orphans of terminal runs, by the process start that accepted each run.
    orphans_by_start: BTreeMap<u64, u64>,
    /// Runs with no revision on the header.
    unattributed: u64,
    /// The `since_deploy` share of each revision.
    revisions: BTreeMap<String, RevisionAgg>,
}

impl SysStatus {
    /// The record as stored, or `None` when the store has none.
    ///
    /// # Errors
    /// Returns [`PiiError`] when the store cannot be read or the record cannot be decoded.
    pub fn read(store: &Store) -> Result<Option<Self>, PiiError> {
        store
            .sys_read(SysTable::Totals, TOTALS_KEY)?
            .map(|bytes| from_bytes(&bytes))
            .transpose()
    }

    /// The two blocks health reports, for the process that started at `process_start_ms` and `revision`.
    #[must_use]
    pub fn counts(&self, process_start_ms: u64, revision: &str) -> (Totals, SinceDeploy) {
        let totals = Totals {
            dispatches_total: self.dispatches_total,
            refusals_total: self.refusals_total,
            criteria_failures_total: self.criteria_failures_total,
            orphans_total: self.orphans_total,
            orphans_since_start: self
                .orphans_by_start
                .get(&process_start_ms)
                .copied()
                .unwrap_or(0),
            agents_in_flight: self.agents_in_flight,
        };
        let share = self.revisions.get(revision).cloned().unwrap_or_default();
        (totals, share.to_deploy(revision, self.unattributed))
    }

    /// Adds `c` to this record, or takes it away when `add` is false.
    fn combine(&mut self, c: &Self, add: bool) {
        bump(&mut self.dispatches_total, c.dispatches_total, add);
        bump(&mut self.refusals_total, c.refusals_total, add);
        bump(
            &mut self.criteria_failures_total,
            c.criteria_failures_total,
            add,
        );
        bump(&mut self.orphans_total, c.orphans_total, add);
        bump(&mut self.agents_in_flight, c.agents_in_flight, add);
        bump(&mut self.unattributed, c.unattributed, add);
        merge_counts(&mut self.orphans_by_start, &c.orphans_by_start, add);
        for (revision, share) in &c.revisions {
            let empty = {
                let slot = self.revisions.entry(revision.clone()).or_default();
                slot.combine(share, add);
                *slot == RevisionAgg::default()
            };
            if empty {
                self.revisions.remove(revision);
            }
        }
    }
}

/// One run's inputs to the record: what its contribution is folded from.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RunRow {
    status: RunStatus,
    process_start_ms: u64,
    revision: String,
    /// Pids spawned and not yet exited, in spawn order.
    open: Vec<i64>,
    dispatches: u64,
    refusals: u64,
    criteria_failures: u64,
    /// The graph's post nodes. Empty when `readable` is false, so no post event counts.
    post_nodes: BTreeSet<NodeId>,
    /// False when the graph body cannot be read or parsed (the replay's "unreadable").
    readable: bool,
    posted: u64,
    post_refused: u64,
}

impl RunRow {
    /// A run with no events yet, for the header that accepted it.
    fn new(store: &Store, header: &RunHeader) -> Self {
        let posts = post_nodes(store, &header.graph);
        Self {
            status: header.status,
            process_start_ms: header.process_start_ms,
            revision: header.revision.clone(),
            open: Vec::new(),
            dispatches: 0,
            refusals: 0,
            criteria_failures: 0,
            readable: posts.is_some(),
            post_nodes: posts.unwrap_or_default(),
            posted: 0,
            post_refused: 0,
        }
    }

    /// What this run adds to the record, given its status and open spawns.
    fn contribution(&self) -> SysStatus {
        let open = self.open.len() as u64;
        let mut c = SysStatus {
            dispatches_total: self.dispatches,
            refusals_total: self.refusals,
            criteria_failures_total: self.criteria_failures,
            ..SysStatus::default()
        };
        match self.status {
            RunStatus::Succeeded | RunStatus::Failed | RunStatus::Refused => {
                c.orphans_total = open;
                if open > 0 {
                    c.orphans_by_start.insert(self.process_start_ms, open);
                }
            }
            RunStatus::Running | RunStatus::Suspended => c.agents_in_flight = open,
        }
        if self.revision.is_empty() {
            c.unattributed = 1;
        } else {
            let posts = self.readable.then_some((self.posted, self.post_refused));
            c.revisions.insert(
                self.revision.clone(),
                RevisionAgg::of_run(self.status, self.process_start_ms, posts),
            );
        }
        c
    }

    /// Folds events in journal order, with the rules of `agent_calls::store_totals` and `since_deploy`.
    fn fold(&mut self, events: &[&RunEvent]) {
        for event in events {
            match event {
                RunEvent::NodeObserved { kind, fields, .. } => {
                    let pid = fields.get("pid").and_then(serde_json::Value::as_i64);
                    match kind.as_str() {
                        "coder.spawned" => {
                            self.dispatches += 1;
                            if let Some(pid) = pid {
                                self.open.push(pid);
                            }
                        }
                        "worker.start.refused" => self.refusals += 1,
                        "coder.exited" => {
                            if fields
                                .get("criteria_failed")
                                .and_then(serde_json::Value::as_i64)
                                .is_some_and(|n| n > 0)
                            {
                                self.criteria_failures += 1;
                            }
                            if let Some(at) =
                                pid.and_then(|p| self.open.iter().position(|o| *o == p))
                            {
                                self.open.remove(at);
                            }
                        }
                        _ => {}
                    }
                }
                RunEvent::NodeFinished { node, value, .. }
                    if self.post_nodes.contains(node)
                        && value.get("duplicate").and_then(serde_json::Value::as_bool)
                            != Some(true) =>
                {
                    self.posted += 1;
                }
                RunEvent::NodeFailed { node, .. } if self.post_nodes.contains(node) => {
                    self.post_refused += 1;
                }
                _ => {}
            }
        }
    }
}

/// The post nodes of `graph`'s stored body, or `None` when the body cannot be read or parsed.
fn post_nodes(store: &Store, graph: &str) -> Option<BTreeSet<NodeId>> {
    let body = store.get_graph(graph).ok()??;
    let spec = GraphSpec::from_json(&body).ok()?;
    Some(
        spec.nodes
            .iter()
            .filter(|n| n.kind == "post")
            .map(|n| n.id.clone())
            .collect(),
    )
}

/// Builds the record from every run in `write` and stores it: the one-time path for a store that has none.
fn rebuild_in(store: &Store, write: &WriteTransaction) -> Result<SysStatus, PiiError> {
    sys_clear(write)?;
    let mut record = SysStatus::default();
    for_each_run_journal(write, |header, events| {
        let mut row = RunRow::new(store, header);
        row.fold(events);
        record.combine(&row.contribution(), true);
        sys_put(write, SysTable::Runs, header.run.as_str(), &to_bytes(&row)?)
    })?;
    sys_put(write, SysTable::Totals, TOTALS_KEY, &to_bytes(&record)?)?;
    Ok(record)
}

/// True when this append is the first event of the store's first run, so a store with no record yet starts one empty
/// rather than waiting for a rebuild. `header` is the header after the append: its seq equals the events appended
/// only when the run had none before.
fn first_run_of_empty_store(
    write: &WriteTransaction,
    header: &RunHeader,
    events: &[&RunEvent],
) -> Result<bool, PiiError> {
    if header.seq != events.len() as u64 {
        return Ok(false);
    }
    Ok(runs_up_to(write, 2)? <= 1)
}

/// The append observer (spec 370): moves the record past `events`.
///
/// It runs inside the append's own transaction, so a failed append moves nothing. While the store has no record it
/// does nothing; the first read rebuilds it from every run.
///
/// # Errors
/// Returns [`PiiError`] when the record or a run row cannot be read or written; the append then aborts.
pub fn observe_append(
    store: &Store,
    write: &WriteTransaction,
    header: &RunHeader,
    events: &[&RunEvent],
) -> Result<(), PiiError> {
    let mut record = match sys_get(write, SysTable::Totals, TOTALS_KEY)? {
        Some(bytes) => from_bytes::<SysStatus>(&bytes)?,
        None if first_run_of_empty_store(write, header, events)? => SysStatus::default(),
        // An older store: its runs are not in the record, so only a rebuild can make it true.
        None => return Ok(()),
    };
    let key = header.run.as_str();
    let mut row = match sys_get(write, SysTable::Runs, key)? {
        Some(prior) => {
            let row: RunRow = from_bytes(&prior)?;
            record.combine(&row.contribution(), false);
            row
        }
        None => RunRow::new(store, header),
    };
    row.status = header.status;
    row.fold(events);
    record.combine(&row.contribution(), true);
    sys_put(write, SysTable::Runs, key, &to_bytes(&row)?)?;
    sys_put(write, SysTable::Totals, TOTALS_KEY, &to_bytes(&record)?)
}

/// The record health reads (spec 370 R2).
///
/// That is the stored one, or, when the store has none, one rebuilt now from every run and stored, returned with
/// `true` so the caller can say so. The record covers every revision, so the process start and revision name the
/// view health reads and are not needed to build it.
///
/// # Errors
/// Returns [`PiiError`] when the store cannot be read or written.
pub fn sys_status_for_health(
    store: &Store,
    _process_start_ms: u64,
    _revision: &str,
) -> Result<(SysStatus, bool), PiiError> {
    if let Some(record) = SysStatus::read(store)? {
        return Ok((record, false));
    }
    let record = store.with_write(|write| rebuild_in(store, write))?;
    Ok((record, true))
}
