//! The scheduler tick engine (spec F4.8, F4.11).
//!
//! [`Engine`] drives the tick loop: propagate dead to a fixpoint, fail
//! loudly on any unsatisfiable join, then drain **all** ready nodes into one
//! batch (never `.pop()`), partition by parallel safety, and apply results as
//! they arrive. Every branch of the loop returns — never a hang.

use std::collections::BTreeMap;
use std::sync::Arc;

use parking_lot::RwLock;

use pii_abi::{NodeId, NodeInput, PiiError, ResumeKey, RunEvent, RunId, RunStatus, Terminal};
use pii_core::{Ctx, Metric};

use crate::blackboard::Blackboard;
use crate::journal::Journaler;
use crate::node::{GraphNode, NodeRegistry};
use crate::run_helpers::{CancelTokenSet, terminal_determination, terminal_status};
use crate::sched::dispatch::{
    DispatchOutcome, Env, FinishedNode, apply, build_node_input, cancel_losers, dispatch,
};
use crate::sched::state::{Readiness, RunState, propagate_dead, readiness};
use crate::spec::{GraphSpec, NodeSpec};
pub(super) struct Engine<'a> {
    pub(super) spec: &'a GraphSpec,
    pub(super) reg: &'a NodeRegistry,
    pub(super) root: &'a Ctx,
    pub(super) run_id: &'a RunId,
    pub(super) bb: &'a Arc<RwLock<Blackboard>>,
    pub(super) jr: &'a Journaler<'a>,
    pub(super) built: &'a BTreeMap<NodeId, Box<dyn GraphNode>>,
    pub(super) tokens: &'a CancelTokenSet,
    /// Nodes the journal shows as still-suspended, keyed to their latest
    /// resume key (spec 30 D1): empty for a fresh `run()`, populated by
    /// `resume()`'s journal replay. Threaded into [`Env`] so `dispatch`
    /// routes these nodes through `on_resume` instead of `run`.
    pub(super) resume_keys: &'a BTreeMap<NodeId, ResumeKey>,
}

impl Engine<'_> {
    /// The tick loop. Every branch returns; there is no unbounded arm.
    // Kept as one function for reviewability of the batch drain and its two
    // terminal arms; splitting would cost more than the line budget saves.
    #[allow(clippy::too_many_lines)]
    pub(super) fn tick_loop(
        &self,
        st: &mut RunState,
        fin: &mut Vec<FinishedNode>,
        failure: &mut Option<String>,
    ) -> Result<Terminal, PiiError> {
        let spec = self.spec;
        let mut step: u32 = 0;
        loop {
            if self.root.cancelled() {
                // Run-level teardown (crash): leave the header Running and
                // flush the write-behind buffer so the run stays discoverable
                // and completed nodes stay durable (resume depends on both —
                // without the flush a pre-threshold cancel would lose the
                // whole journal and completed work would re-run, duplicating
                // side effects).
                return self.cancelled();
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
            if self
                .root
                .budget()
                .remaining(pii_core::BudgetKind::ToolCalls)
                == 0
            {
                return self.finish(
                    &Terminal::Failure {
                        error: "budget exhausted: ToolCalls".into(),
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
            // F4.5 budget enforcement at dispatch time: a budgeted node that
            // has already run `budget` times must not run again — the run
            // fails instead (exactly `budget` executions, then terminal).
            if let Some(n) = batch.iter().find(|n| {
                n.loop_budget.is_some()
                    && st.iters.get(&n.id).copied().unwrap_or(0) >= n.loop_budget.unwrap_or(0)
            }) {
                let terminal = Terminal::Failure {
                    error: format!("loop_budget exceeded for node '{}'", n.id),
                };
                return self.finish(&terminal, RunStatus::Failed);
            }
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
                resume_keys: self.resume_keys,
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
                    // Perf/RSS (opus review, 2026-09-05): a one-element chunk
                    // has no parallelism to win, and `std::thread::scope` +
                    // `s.spawn` still create and join a real OS thread for
                    // it. Under high run-count soak load that showed up as
                    // glibc allocator retention (per-thread arenas + a
                    // cached stack per churned thread) masquerading as an
                    // RSS leak in `xtask rss-gate` — flat fd count, no
                    // growing collection, but climbing RSS. Dispatch inline
                    // instead: behaviour-identical to the loop below (same
                    // `apply`/`cancel_losers` order), zero threads spawned.
                    if let [only] = chunk {
                        if self.root.cancelled() {
                            return self.cancelled();
                        }
                        let out = dispatch(&env, only);
                        if let Some(key) = apply(&env, st, fin, failure, &only.id, out) {
                            suspended = Some(key);
                        }
                        cancel_losers(&env, st);
                        if suspended.is_some() {
                            break;
                        }
                        continue;
                    }
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
                            // Terminal failure recorded (e.g. loop_budget):
                            // stop draining this batch — nothing further may run.
                            if failure.is_some() {
                                break;
                            }
                        }
                    });
                    if self.root.cancelled() {
                        return self.cancelled();
                    }
                    if suspended.is_some() {
                        break;
                    }
                }
            }
            if suspended.is_none() {
                for n in &ser {
                    if self.root.cancelled() {
                        return self.cancelled();
                    }
                    let out = dispatch(&env, n);
                    if let Some(key) = apply(&env, st, fin, failure, &n.id, out) {
                        suspended = Some(key);
                        break;
                    }
                    cancel_losers(&env, st);
                    // Terminal failure recorded (e.g. loop_budget): stop the
                    // serial queue — nothing further may run.
                    if failure.is_some() {
                        break;
                    }
                }
            }
            // A node in this tick's batch may have hit a terminal failure
            // (e.g. loop_budget exceeded — F4.5). Terminate NOW rather than
            // dispatching another batch: the failure was recorded mid-batch,
            // and continuing would let nodes that are ready because of this
            // batch's completions run once more (observable as a budgeted
            // cycle executing one extra iteration). Deterministic: exactly
            // `budget` executions of the budgeted node, then terminal.
            if let Some(err) = failure.as_deref() {
                return self.finish(
                    &Terminal::Failure {
                        error: err.to_string(),
                    },
                    RunStatus::Failed,
                );
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

    /// Run-level teardown (crash/cancel): flush the write-behind journal so
    /// the run stays discoverable and its completed nodes stay durable
    /// (resume depends on both), leave the header `Running` for recovery,
    /// then return `Cancelled`. Every cancel exit funnels through here.
    fn cancelled(&self) -> Result<Terminal, PiiError> {
        let _ = self.jr.flush();
        Err(PiiError::Cancelled)
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
        self.jr.set_dur_ms(self.root.elapsed_ms());
        // Append the terminal event BEFORE flipping status: `set_status`
        // flushes immediately, and if it ran first while other events were
        // still pending, that flush would persist the new (non-Running)
        // status without the `Finished` event in the same transaction —
        // a reader polling `get_run` would see the run as done and read the
        // journal before the terminal event landed (reattach/bus race).
        // Buffering `Finished` first means `set_status`'s flush carries both
        // the status transition and the terminal event atomically.
        self.jr.append(&RunEvent::Finished {
            terminal: terminal.clone(),
            output_hash,
        })?;
        self.jr.set_status(status);
        // Spec 11 J2 durability barrier: the terminal event must be durable
        // before run()/resume() returns — every post-run journal reader
        // (trace, tests, reattach, bus) depends on a complete journal.
        self.jr.flush()?;
        Ok(terminal.clone())
    }
}
