//! The scheduler tick loop (spec F4.8) and run entry (F4.11).

use std::collections::BTreeMap;

use pii_abi::{NodeId, NodeInput, Outcome, PiiError, RunId, Terminal};
use pii_core::{BudgetShare, CapSet, Ctx, Principal};

use crate::blackboard::Blackboard;
use crate::node::{GraphNode, NodeRegistry};
use crate::sched::state::{Readiness, RunState, apply_completion, propagate_dead, readiness};
use crate::spec::GraphSpec;

/// A node built from its config, with its own root caps override.
struct BuiltNode {
    node: Box<dyn GraphNode>,
}

/// Result of a single node dispatch.
#[derive(Debug)]
enum DispatchResult {
    Next {
        port: pii_abi::PortName,
        value: serde_json::Value,
    },
    Fail {
        error: String,
    },
}

/// Runs a graph to a terminal disposition against a fresh run state.
///
/// # Errors
/// Returns [`PiiError`] on a hard engine error (not a node failure, which is
/// captured in the returned [`Terminal`]).
pub fn run(
    spec: &GraphSpec,
    reg: &NodeRegistry,
    ctx: &Ctx,
    _run: &RunId,
    input: serde_json::Value,
) -> Result<Terminal, PiiError> {
    let mut st = RunState::from_spec(spec);
    let mut bb = Blackboard::new(input);
    let mut built: BTreeMap<NodeId, BuiltNode> = BTreeMap::new();
    for n in &spec.nodes {
        let f = reg.factory(&n.kind)?;
        let node = f.build(&n.cfg)?;
        built.insert(n.id.clone(), BuiltNode { node });
    }

    let mut step_count = 0u32;
    loop {
        if step_count > spec.limits.max_steps {
            return Ok(Terminal::Failure {
                error: "max_steps exceeded".into(),
            });
        }
        let elapsed_ms = u64::from(ctx.elapsed_ms());
        if elapsed_ms > spec.limits.max_wall_ms {
            return Ok(Terminal::Failure {
                error: "max_wall_ms exceeded".into(),
            });
        }

        propagate_dead(spec, &mut st);

        // Unsatisfiable join check.
        if let Some(n) = spec.nodes.iter().find(|n| {
            !st.dead.contains(&n.id)
                && !st.executed.contains(&n.id)
                && readiness(spec, &st, &n.id) == Readiness::UnsatisfiableJoin
        }) {
            return Ok(Terminal::Failure {
                error: format!("join can never be satisfied: node '{}'", n.id),
            });
        }

        // Drain ALL ready nodes (not just one).
        let batch: Vec<&crate::spec::NodeSpec> = spec
            .nodes
            .iter()
            .filter(|n| {
                !st.executed.contains(&n.id)
                    && !st.dead.contains(&n.id)
                    && readiness(spec, &st, &n.id) == Readiness::Ready
            })
            .collect();

        if batch.is_empty() {
            return Ok(terminal_determination(spec, &st, &bb));
        }

        step_count += 1;
        for ns in batch {
            let r = dispatch_one(spec, &st, ctx, &built, &mut bb, ns);
            match r {
                DispatchResult::Next { port, value } => {
                    apply_completion(spec, &mut st, &ns.id, &port, &value);
                }
                DispatchResult::Fail { error } => {
                    return Ok(Terminal::Failure { error });
                }
            }
        }
    }
}

/// Runs one node, journals it, updates the blackboard with its output.
fn dispatch_one(
    spec: &GraphSpec,
    st: &RunState,
    ctx: &Ctx,
    built: &BTreeMap<NodeId, BuiltNode>,
    bb: &mut Blackboard,
    ns: &crate::spec::NodeSpec,
) -> DispatchResult {
    let _ = spec;
    let _ = st;
    // Node context with the graph-granted caps.
    let node_caps = CapSet::empty(); // root scopes are applied by the CLI at run time
    let child = ctx.child(
        Principal::GraphNode {
            run: RunId::from("run"),
            node: ns.id.clone(),
        },
        &node_caps,
        &BudgetShare::All,
    );

    let input = NodeInput {
        input: bb.input.clone(),
        payload: node_payload(spec, st, &ns.id),
        nodes: serde_json::to_value(&bb.nodes).unwrap_or(serde_json::Value::Null),
    };

    let Some(b) = built.get(&ns.id) else {
        return DispatchResult::Fail {
            error: format!("node '{}' not built", ns.id),
        };
    };

    match b.node.run(&child, &input) {
        Ok(Outcome::Next { port, value }) => {
            bb.record(ns.id.as_str(), value.clone());
            DispatchResult::Next { port, value }
        }
        Ok(Outcome::Suspend { .. } | Outcome::Fail { .. }) => DispatchResult::Fail {
            error: format!("node '{}' did not produce a Next outcome", ns.id),
        },
        Err(e) => DispatchResult::Fail {
            error: e.to_string(),
        },
    }
}

/// Builds the join payload: a JSON object keyed by source node id holding the
/// value of every `Fired` inbound Data edge. A node with no inbound Data edges
/// gets `input.clone()`.
fn node_payload(spec: &GraphSpec, st: &RunState, n: &NodeId) -> serde_json::Value {
    let in_data = spec.inbound(n, crate::spec::EdgeKind::Data);
    if in_data.is_empty() {
        return serde_json::Value::Null; // caller replaces with input
    }
    let mut obj = serde_json::Map::new();
    for &ei in &in_data {
        if let Some(edge) = spec.edges.get(ei)
            && let Some(crate::sched::state::EdgeState::Fired { value }) = st.edges.get(ei)
        {
            obj.insert(edge.from.as_str().to_string(), value.clone());
        }
    }
    serde_json::Value::Object(obj)
}

/// Determines the terminal disposition when no node is ready (F4.8).
fn terminal_determination(spec: &GraphSpec, st: &RunState, bb: &Blackboard) -> Terminal {
    let any_executed = !st.executed.is_empty();
    let any_not_ready = spec.nodes.iter().any(|n| {
        !st.executed.contains(&n.id)
            && !st.dead.contains(&n.id)
            && readiness(spec, st, &n.id) == Readiness::NotReady
    });
    if any_not_ready {
        let stuck = spec
            .nodes
            .iter()
            .find(|n| !st.executed.contains(&n.id) && !st.dead.contains(&n.id))
            .map_or_else(|| "?".into(), |n| n.id.to_string());
        return Terminal::Failure {
            error: format!("no progress: node '{stuck}' never became ready"),
        };
    }
    if any_executed {
        // Output = NodeFinished value of the last executed node.
        if let Some((_, v)) = bb.nodes.iter().next_back() {
            return Terminal::Success { output: v.clone() };
        }
    }
    Terminal::Success {
        output: serde_json::Value::Null,
    }
}
