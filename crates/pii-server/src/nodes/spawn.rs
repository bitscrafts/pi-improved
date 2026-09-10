//! `spawn` node kind (spec 32 R1): launches a child run on the same engine.
//!
//! Lives in `pii-server` (not `pii-graph`'s builtins) because it needs the
//! `Engine` to call `start_run`. The factory carries a shared
//! `Arc<Mutex<Option<Arc<Engine>>>>` handle (the `worker` node's pattern) that
//! the daemon (or a test) fills via `Engine::attach_spawn_engine` after
//! construction — the registry is built inside `Engine::new`, so the factory
//! cannot hold the `Arc<Engine>` directly (chicken-and-egg).
//!
//! Config: `{ graph: String, input: Value }` where `input` may contain
//! `$.`-expressions resolved against the node's eval context. Ports `ok`/`err`.
//! On `ok` the value is the child `run_id` string, and a `RunSpawned` journal
//! event links the parent→child edge for `pii graph trace`.

use std::sync::Arc;

use pii_abi::{Effects, NodeInput, Outcome, PiiError, PortName, RunEvent, RunId};
use pii_core::{CapKind, Ctx};
use pii_graph::expr::eval_or_literal;
use pii_graph::node::{GraphNode, NodeFactory};

/// Shared handle to the engine the `spawn` node launches child runs on.
/// `None` until the daemon (or a test) calls `Engine::attach_spawn_engine`.
pub type SpawnEngineHandle = Arc<parking_lot::Mutex<Option<Arc<crate::Engine>>>>;

/// Config: `{ graph: String, input: Value }`.
pub struct SpawnNode {
    engine: SpawnEngineHandle,
    graph: String,
    input: serde_json::Value,
}

impl GraphNode for SpawnNode {
    fn run(&self, _ctx: &Ctx, input: &NodeInput) -> Result<Outcome, PiiError> {
        let ec = serde_json::json!({
            "input": input.input,
            "payload": input.payload,
            "nodes": input.nodes,
        });
        let resolved = resolve(&ec, &self.input)?;
        let engine = self
            .engine
            .lock()
            .clone()
            .ok_or_else(|| PiiError::Unavailable("spawn: engine handle not attached".into()))?;
        // R5 (spec 32): refuse to spawn a heal for a signature whose
        // attempt_count >= 2 — route to the escalation port instead.
        if let Some(sig) = resolved
            .get("signature")
            .and_then(serde_json::Value::as_str)
            && let Some(attempt) = engine.heals.get_attempt(sig)?
            && attempt.attempt_count >= 2
        {
            return Ok(Outcome::Next {
                port: PortName::from("escalate"),
                value: serde_json::json!({
                    "error": "attempt_count >= 2, refusing to spawn another heal",
                    "signature": sig,
                    "attempt_count": attempt.attempt_count,
                }),
            });
        }
        let child = engine.start_run(&self.graph, &resolved)?;
        Ok(Outcome::Next {
            port: PortName::from("ok"),
            value: serde_json::json!({ "run_id": child.to_string() }),
        })
    }

    /// The parent→child `RunSpawned` link, journaled alongside this node's
    /// own `NodeFinished` (spec 32 R1).
    fn link_events(&self, value: &serde_json::Value) -> Vec<RunEvent> {
        let child = value
            .get("run_id")
            .and_then(serde_json::Value::as_str)
            .map(RunId::from);
        child.map_or_else(Vec::new, |c| vec![RunEvent::RunSpawned { child_run: c }])
    }
}

/// Factory carrying the shared engine handle.
pub struct SpawnFactory {
    /// The engine handle (empty outside the daemon / until attached).
    pub engine: SpawnEngineHandle,
}

impl NodeFactory for SpawnFactory {
    fn kind(&self) -> &'static str {
        "spawn"
    }

    fn effects(&self) -> Effects {
        Effects::MUTATES_EXTERNAL
    }

    fn required_caps(&self) -> CapKind {
        CapKind::SPAWN
    }

    fn ports(&self, _cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError> {
        Ok(vec![
            PortName::from("ok"),
            PortName::from("err"),
            PortName::from("escalate"),
        ])
    }

    fn build(&self, cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError> {
        let graph = cfg
            .get("graph")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| PiiError::Invalid("spawn node requires 'graph' string".into()))?
            .to_owned();
        let input = cfg.get("input").cloned().unwrap_or(serde_json::Value::Null);
        Ok(Box::new(SpawnNode {
            engine: Arc::clone(&self.engine),
            graph,
            input,
        }))
    }
}

/// Recursively resolves `$.`-string leaves in `input` (mirror of the tool node).
fn resolve(ctx: &serde_json::Value, v: &serde_json::Value) -> Result<serde_json::Value, PiiError> {
    match v {
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, val) in map {
                out.insert(k.clone(), resolve(ctx, val)?);
            }
            Ok(serde_json::Value::Object(out))
        }
        serde_json::Value::Array(items) => {
            let mut out = Vec::new();
            for item in items {
                out.push(resolve(ctx, item)?);
            }
            Ok(serde_json::Value::Array(out))
        }
        other => eval_or_literal(ctx, other),
    }
}
