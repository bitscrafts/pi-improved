//! Store-backed run entry points (spec F4.8, F4.11) and the scheduler tick.
//!
//! One scheduler tick (pass-3 §4.3): propagate dead to a fixpoint, fail
//! loudly on any unsatisfiable join, then drain **all** ready nodes into one
//! batch (never `.pop()`), partition by parallel safety, and apply results as
//! they arrive. Every branch of the loop returns — never a hang.
//!
//! `run` journals every event to the [`pii_store::Store`] and snapshots the
//! blackboard every 32 events (see [`crate::journal`]). `resume` replays the
//! whole journal to rebuild the state — a node with `NodeFinished` is never
//! re-invoked (its value is in the journal); a node with `NodeStarted` and no
//! terminal event **is** re-run, which is the documented Phase-1 boundary.
//! Cyclic execution also lands in Phase 2: `run`/`resume` reject any cycle
//! with [`PiiError::Invalid`] even when `loop_budget` makes it lint-clean.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use parking_lot::RwLock;

use pii_abi::{NodeId, NodeInput, PiiError, ResumeKey, RunEvent, RunId, RunStatus, Terminal};
use pii_core::{Ctx, Metric};
use pii_store::Store;

use crate::blackboard::Blackboard;
use crate::journal::Journaler;
use crate::node::{GraphNode, NodeRegistry};
use crate::sched::dispatch::{
    DispatchOutcome, Env, FinishedNode, apply, build_node_input, cancel_losers, dispatch,
};
use crate::sched::state::{Readiness, RunState, propagate_dead, readiness};
use crate::spec::{EdgeKind, GraphSpec, NodeSpec};
use crate::validate::validate;

/// Runs a graph to a terminal disposition, journaling every event to the
/// store under `run_id`.
///
/// # Errors
/// Returns [`PiiError::Invalid`] for cyclic specs (Phase-1 boundary) or node
/// build failures, [`PiiError::Cancelled`] when the root context is torn down
/// mid-run, and store errors from journaling.
pub fn run(
    store: &Store,
    spec: &GraphSpec,
    reg: &NodeRegistry,
    ctx: &Ctx,
    run_id: &RunId,
    input: serde_json::Value,
) -> Result<Terminal, PiiError> {
    run_with_validate(store, spec, reg, ctx, run_id, input, true)
}

/// [`run`] with an explicit validation switch.
///
/// `validate` defaults to `true` (the library always validates unless a caller
/// opts out). The CLI's `--no-validate` passes `false` so the runtime
/// `UnsatisfiableJoin` path (pass-3 Test C) can be exercised on a graph that
/// V6 rejects at load time.
///
/// # Errors
/// Returns [`PiiError::Invalid`] for cyclic specs (Phase-1 boundary), node
/// build failures, or (when `validate` is true) load-time validation errors;
/// [`PiiError::Cancelled`] when the root context is torn down mid-run; and
/// store errors from journaling.
pub fn run_with_validate(
    store: &Store,
    spec: &GraphSpec,
    reg: &NodeRegistry,
    ctx: &Ctx,
    run_id: &RunId,
    input: serde_json::Value,
    validate: bool,
) -> Result<Terminal, PiiError> {
    if spec_has_cycle(spec) {
        return Err(PiiError::Invalid(PHASE2_CYCLE.to_string()));
    }
    if validate {
        validate_for_exec(spec, reg)?;
    }
    let built = build_nodes(spec, reg)?;
    let spec_json = spec.to_json()?;
    let graph_hash = store.put_graph(&spec_json)?;
    // The run input is stored content-addressed; `Started.input_hash` is the
    // key resume uses to recover it without reading snapshots.
    let input_bytes =
        serde_json::to_vec(&input).map_err(|e| PiiError::Internal(format!("input serde: {e}")))?;
    let input_hash = store.put_graph(&input_bytes)?;

    let bb = Arc::new(RwLock::new(Blackboard::new(input)));
    let jr = Journaler::fresh(
        store,
        ctx.observer().clone(),
        run_id.clone(),
        graph_hash,
        Arc::clone(&bb),
    );
    jr.append(&RunEvent::Started {
        graph: spec.name.clone(),
        input_hash,
    })?;

    let tokens = fresh_tokens(ctx, spec);
    let mut st = RunState::from_spec(spec);
    let mut fin = Vec::new();
    let mut failure = None;
    let engine = Engine {
        spec,
        reg,
        root: ctx,
        run_id,
        bb: &bb,
        jr: &jr,
        built: &built,
        tokens: &tokens,
    };
    engine.tick_loop(&mut st, &mut fin, &mut failure)
}

/// Resumes a run by replaying its journal, then continuing the tick loop.
///
/// # Errors
/// Returns [`PiiError::NotFound`] for an unknown run or graph body,
/// [`PiiError::Invalid`] for a cyclic spec, and store errors from journaling.
pub fn resume(
    store: &Store,
    reg: &NodeRegistry,
    ctx: &Ctx,
    run_id: &RunId,
) -> Result<Terminal, PiiError> {
    let header = store
        .get_run(run_id)?
        .ok_or_else(|| PiiError::NotFound(format!("run '{run_id}' not found")))?;
    let spec_bytes = store
        .get_graph(&header.graph)?
        .ok_or_else(|| PiiError::NotFound(format!("graph body '{}'", header.graph)))?;
    let spec = GraphSpec::from_json(&spec_bytes)?;
    if spec_has_cycle(&spec) {
        return Err(PiiError::Invalid(PHASE2_CYCLE.to_string()));
    }
    validate_for_exec(&spec, reg)?;
    let journal = store.read_journal(run_id)?;

    // Recover the run input from the content-addressed store.
    let input = journal
        .iter()
        .find_map(|(_, ev)| match ev {
            RunEvent::Started { input_hash, .. } => store
                .get_graph(input_hash)
                .ok()
                .flatten()
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok()),
            _ => None,
        })
        .unwrap_or(serde_json::Value::Null);

    let bb = Arc::new(RwLock::new(Blackboard::new(input.clone())));
    let jr = Journaler::adopt(store, ctx.observer().clone(), header, Arc::clone(&bb));

    // Replay the journal: rebuild state, blackboard, and completion order.
    let mut st = RunState::from_spec(&spec);
    let mut fin: Vec<FinishedNode> = Vec::new();
    let mut failure: Option<String> = None;
    replay(&journal, &spec, &mut st, &bb, &mut fin, &mut failure);

    let payload_bytes =
        serde_json::to_vec(&input).map_err(|e| PiiError::Internal(format!("input serde: {e}")))?;
    jr.append(&RunEvent::Resumed {
        key: ResumeKey(String::new()),
        payload_hash: pii_store::hash(&payload_bytes),
    })?;

    // Nodes killed by the final (unjournalled) propagation before the crash.
    let newly = propagate_dead(&spec, &mut st);
    for id in &newly {
        jr.append(&RunEvent::NodeDead { node: id.clone() })?;
    }

    let built = build_nodes(&spec, reg)?;
    let tokens = fresh_tokens(ctx, &spec);
    let engine = Engine {
        spec: &spec,
        reg,
        root: ctx,
        run_id,
        bb: &bb,
        jr: &jr,
        built: &built,
        tokens: &tokens,
    };
    engine.tick_loop(&mut st, &mut fin, &mut failure)
}

/// The documented Phase-1 boundary (scope decision 3, part-2 spec).
const PHASE2_CYCLE: &str = "cyclic execution lands in Phase 2 (the Phase-1 scheduler executes DAGs only; \
     loop_budget permits a bounded cycle at validate time, not at run time)";

struct Engine<'a> {
    spec: &'a GraphSpec,
    reg: &'a NodeRegistry,
    root: &'a Ctx,
    run_id: &'a RunId,
    bb: &'a Arc<RwLock<Blackboard>>,
    jr: &'a Journaler<'a>,
    built: &'a BTreeMap<NodeId, Box<dyn GraphNode>>,
    tokens: &'a CancelTokenSet,
}

type CancelTokenSet = BTreeMap<NodeId, pii_core::CancelToken>;

impl Engine<'_> {
    /// The tick loop. Every branch returns; there is no unbounded arm.
    // Kept as one function for reviewability of the batch drain and its two
    // terminal arms; splitting would cost more than the line budget saves.
    #[allow(clippy::too_many_lines)]
    fn tick_loop(
        &self,
        st: &mut RunState,
        fin: &mut Vec<FinishedNode>,
        failure: &mut Option<String>,
    ) -> Result<Terminal, PiiError> {
        let spec = self.spec;
        let mut step: u32 = 0;
        loop {
            if self.root.cancelled() {
                // Run-level teardown (crash): leave the header Running.
                return Err(PiiError::Cancelled);
            }
            if step > spec.limits.max_steps {
                return self.finish(
                    &Terminal::Failure {
                        error: "max_steps exceeded".into(),
                    },
                    RunStatus::Failed,
                );
            }
            if u64::from(self.root.elapsed_ms()) > spec.limits.max_wall_ms {
                return self.finish(
                    &Terminal::Failure {
                        error: "max_wall_ms exceeded".into(),
                    },
                    RunStatus::Failed,
                );
            }
            // Budget is checked before dispatch (pass-1 §3.5): a run whose
            // token or micro-dollar budget is exhausted is terminal and
            // non-retryable. Nodes share the run's budget (BudgetShare::Shared),
            // so a charge by any node is visible here on the next tick.
            if self.root.budget().remaining(pii_core::BudgetKind::Tokens) == 0 {
                return self.finish(
                    &Terminal::Failure {
                        error: "budget exhausted: Tokens".into(),
                    },
                    RunStatus::Failed,
                );
            }
            if self.root.budget().remaining(pii_core::BudgetKind::MicroUsd) == 0 {
                return self.finish(
                    &Terminal::Failure {
                        error: "budget exhausted: MicroUsd".into(),
                    },
                    RunStatus::Failed,
                );
            }

            // Dead propagation to a fixpoint, journaling each new death.
            let newly = propagate_dead(spec, st);
            if !newly.is_empty() {
                self.metric("graph.node.dead", newly.len() as u64);
                for id in &newly {
                    self.jr.append(&RunEvent::NodeDead { node: id.clone() })?;
                }
            }

            // Unsatisfiable join: LOUD, immediate, named (within one tick).
            if let Some(n) = self.unsatisfiable(st) {
                self.metric("graph.join.unsatisfiable", 1);
                let terminal = Terminal::Failure {
                    error: format!("join can never be satisfied: node '{}'", n.id),
                };
                return self.finish(&terminal, RunStatus::Failed);
            }

            // Drain ALL ready nodes into one batch — never .pop()/.next().
            // Failed and cancelled nodes are settled terminal states: they
            // are never re-dispatched (a terminally failed node re-running
            // every tick would burn max_steps; a cancelled race loser would
            // wake up and finish its full timer).
            let batch: Vec<&NodeSpec> = spec
                .nodes
                .iter()
                .filter(|n| {
                    !st.executed.contains(&n.id)
                        && !st.dead.contains(&n.id)
                        && !st.failed.contains(&n.id)
                        && !st.cancelled.contains(&n.id)
                        && readiness(spec, st, &n.id) == Readiness::Ready
                })
                .collect();
            if batch.is_empty() {
                let terminal = terminal_determination(spec, st, fin, failure.as_deref());
                let status = terminal_status(&terminal);
                return self.finish(&terminal, status);
            }

            let ids: Vec<NodeId> = batch.iter().map(|n| n.id.clone()).collect();
            let width = u32::try_from(batch.len()).unwrap_or(u32::MAX);
            self.metric("graph.batch.width", u64::from(width));
            self.jr
                .append(&RunEvent::TickStarted { batch: ids, width })?;

            // Freeze this tick's inputs from the current state + blackboard.
            let mut inputs: BTreeMap<NodeId, NodeInput> = BTreeMap::new();
            {
                let bb_read = self.bb.read();
                for n in &batch {
                    inputs.insert(n.id.clone(), build_node_input(spec, st, &bb_read, &n.id));
                }
            }
            let env = Env {
                spec,
                reg: self.reg,
                built: self.built,
                root: self.root,
                tokens: self.tokens,
                inputs: &inputs,
                jr: self.jr,
                run: self.run_id,
                bb: self.bb,
            };

            // Partition by parallel safety (F4.8): Pure | ReadsFs parallel,
            // WritesFs | MutatesExternal sequential.
            let mut par: Vec<&NodeSpec> = Vec::new();
            let mut ser: Vec<&NodeSpec> = Vec::new();
            for n in &batch {
                let safe = self
                    .reg
                    .factory(&n.kind)
                    .is_ok_and(|f| f.effects().is_parallel_safe());
                if safe {
                    par.push(n);
                } else {
                    ser.push(n);
                }
            }

            let mut suspended: Option<ResumeKey> = None;

            if !par.is_empty() {
                // Bound the OS-thread fan-out (pass-1 §3.5): a wide PURE/READS_FS
                // batch must not spawn one thread per node. Chunk by
                // max_parallel (or available_parallelism() when 0), running
                // each chunk's threads in a scope and applying results as they
                // arrive (cancel_losers after EACH result, per F4.9).
                let limit = if spec.limits.max_parallel == 0 {
                    std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get)
                } else {
                    spec.limits.max_parallel as usize
                };
                for chunk in par.chunks(limit.max(1)) {
                    let (tx, rx) = std::sync::mpsc::channel::<(NodeId, DispatchOutcome)>();
                    std::thread::scope(|s| {
                        for n in chunk {
                            let tx = tx.clone();
                            let env = &env;
                            s.spawn(move || {
                                let out = dispatch(env, n);
                                let _ = tx.send((n.id.clone(), out));
                            });
                        }
                        drop(tx);
                        // Apply results as they arrive; cancel losers after EACH.
                        for (node, out) in rx {
                            if let Some(key) = apply(&env, st, fin, failure, &node, out) {
                                suspended = Some(key);
                            }
                            cancel_losers(&env, st);
                        }
                    });
                    if self.root.cancelled() {
                        return Err(PiiError::Cancelled);
                    }
                    if suspended.is_some() {
                        break;
                    }
                }
            }
            if suspended.is_none() {
                for n in &ser {
                    if self.root.cancelled() {
                        return Err(PiiError::Cancelled);
                    }
                    let out = dispatch(&env, n);
                    if let Some(key) = apply(&env, st, fin, failure, &n.id, out) {
                        suspended = Some(key);
                        break;
                    }
                    cancel_losers(&env, st);
                }
            }
            step += 1;

            if let Some(key) = suspended {
                return self.finish(&Terminal::Suspended { key }, RunStatus::Suspended);
            }
        }
    }

    fn metric(&self, name: &str, value: u64) {
        self.root.observer().metric(&Metric {
            name: String::from(name),
            value,
        });
    }

    fn unsatisfiable(&self, st: &RunState) -> Option<&NodeSpec> {
        self.spec.nodes.iter().find(|n| {
            !st.executed.contains(&n.id)
                && !st.dead.contains(&n.id)
                && !st.failed.contains(&n.id)
                && !st.cancelled.contains(&n.id)
                && readiness(self.spec, st, &n.id) == Readiness::UnsatisfiableJoin
        })
    }

    /// Journals the `Finished` event with the terminal status and returns.
    fn finish(&self, terminal: &Terminal, status: RunStatus) -> Result<Terminal, PiiError> {
        let bytes = serde_json::to_vec(terminal)
            .map_err(|e| PiiError::Internal(format!("terminal serde: {e}")))?;
        let output_hash = pii_store::hash(&bytes);
        self.jr.set_status(status);
        self.jr.set_dur_ms(self.root.elapsed_ms());
        self.jr.append(&RunEvent::Finished {
            terminal: terminal.clone(),
            output_hash,
        })?;
        Ok(terminal.clone())
    }
}

/// Runs `validate()` and maps any errors to a single `PiiError::Invalid`.
///
/// The engine must never store or execute a graph that fails load-time
/// validation (pass-1 §3.2: *"`validate()` runs before a graph can be stored or
/// executed"*). The CLI's `--no-validate` skips its own pre-flight only; the
/// library entry points always validate.
fn validate_for_exec(spec: &GraphSpec, reg: &NodeRegistry) -> Result<(), PiiError> {
    if let Err(errors) = validate(spec, reg) {
        return Err(PiiError::Invalid(format!(
            "graph failed validation: {}",
            errors.join("; ")
        )));
    }
    Ok(())
}

/// Builds every node instance up front (build errors surface before journalling).
fn build_nodes(
    spec: &GraphSpec,
    reg: &NodeRegistry,
) -> Result<BTreeMap<NodeId, Box<dyn GraphNode>>, PiiError> {
    let mut built = BTreeMap::new();
    for n in &spec.nodes {
        let factory = reg.factory(&n.kind)?;
        let node = factory.build(&n.cfg)?;
        built.insert(n.id.clone(), node);
    }
    Ok(built)
}

fn fresh_tokens(ctx: &Ctx, spec: &GraphSpec) -> CancelTokenSet {
    spec.nodes
        .iter()
        .map(|n| (n.id.clone(), ctx.cancel_token().child()))
        .collect()
}

/// Replays a journal into the state (spec F4.11). Nodes that eventually
/// finished have their failed attempts skipped; a `NodeStarted` without a
/// terminal event is simply ignored — that node re-runs.
fn replay(
    journal: &[(u64, RunEvent)],
    spec: &GraphSpec,
    st: &mut RunState,
    bb: &Arc<RwLock<Blackboard>>,
    fin: &mut Vec<FinishedNode>,
    failure: &mut Option<String>,
) {
    let finished: BTreeSet<NodeId> = journal
        .iter()
        .filter_map(|(_, ev)| match ev {
            RunEvent::NodeFinished { node, .. } => Some(node.clone()),
            _ => None,
        })
        .collect();
    for (_, ev) in journal {
        match ev {
            RunEvent::NodeFinished {
                node, port, value, ..
            } => {
                crate::sched::state::apply_completion(spec, st, node, port, value);
                bb.write().record(node.as_str(), value.clone());
                fin.push(FinishedNode {
                    node: node.clone(),
                    port: port.clone(),
                    value: value.clone(),
                });
            }
            RunEvent::NodeDead { node } => {
                st.dead.insert(node.clone());
                crate::sched::state::apply_failed_or_cancelled(spec, st, node);
            }
            RunEvent::NodeCancelled { node } => {
                st.cancelled.insert(node.clone());
                crate::sched::state::apply_failed_or_cancelled(spec, st, node);
            }
            RunEvent::NodeFailed {
                node,
                error,
                retryable: false,
                ..
            } if !finished.contains(node) && failure.is_none() => {
                // A resolved node's failed attempts are history, not state.
                st.failed.insert(node.clone());
                crate::sched::state::apply_failed_or_cancelled(spec, st, node);
                *failure = Some(error.clone());
            }
            _ => {}
        }
    }
}

/// Terminal disposition when no node is ready (spec F4.8).
fn terminal_determination(
    spec: &GraphSpec,
    st: &RunState,
    fin: &[FinishedNode],
    failure: Option<&str>,
) -> Terminal {
    if let Some(err) = failure {
        return Terminal::Failure {
            error: err.to_string(),
        };
    }
    if let Some(n) = spec.nodes.iter().find(|n| {
        !st.executed.contains(&n.id)
            && !st.dead.contains(&n.id)
            && !st.failed.contains(&n.id)
            && !st.cancelled.contains(&n.id)
            && readiness(spec, st, &n.id) == Readiness::NotReady
    }) {
        return Terminal::Failure {
            error: format!("no progress: node '{}' never became ready", n.id),
        };
    }
    // Output = the latest finished node with no outbound Data edge on the
    // port it took (latest by seq if several).
    for f in fin.iter().rev() {
        let has_out = spec.outbound(&f.node).iter().any(|&ei| {
            spec.edges
                .get(ei)
                .is_some_and(|e| e.kind == EdgeKind::Data && e.port == f.port)
        });
        if !has_out {
            return Terminal::Success {
                output: f.value.clone(),
            };
        }
    }
    if let Some(last) = fin.last() {
        return Terminal::Success {
            output: last.value.clone(),
        };
    }
    Terminal::Success {
        output: serde_json::Value::Null,
    }
}

fn terminal_status(terminal: &Terminal) -> RunStatus {
    match terminal {
        Terminal::Success { .. } => RunStatus::Succeeded,
        Terminal::Failure { .. } => RunStatus::Failed,
        Terminal::Suspended { .. } => RunStatus::Suspended,
    }
}

/// True when the spec contains a cycle (SCC with >1 node, or a self-loop).
#[must_use]
pub fn spec_has_cycle(spec: &GraphSpec) -> bool {
    let mut g = petgraph::graph::DiGraph::new();
    let mut idx: BTreeMap<NodeId, petgraph::graph::NodeIndex> = BTreeMap::new();
    for n in &spec.nodes {
        idx.insert(n.id.clone(), g.add_node(()));
    }
    for e in &spec.edges {
        if let (Some(&a), Some(&b)) = (idx.get(&e.from), idx.get(&e.to)) {
            g.add_edge(a, b, ());
        }
    }
    petgraph::algo::tarjan_scc(&g)
        .iter()
        .any(|scc| scc.len() > 1 || scc.first().is_some_and(|&n| g.find_edge(n, n).is_some()))
}
