//! `worker` node kind (F3.7): invokes a supervised worker through the
//! daemon's supervisor map. Lives in `pii-server` (not `pii-graph`'s
//! builtins) because only the daemon owns supervisors. An `Unavailable`
//! error (dead worker mid-flight) propagates UNCHANGED so the scheduler's
//! retry path sees `is_retryable() == true` and retries — the F3.7 contract.

use std::sync::Arc;

use pii_abi::{Effects, NodeInput, Outcome, PiiError, PortName};
use pii_core::{CapKind, Ctx};
use pii_graph::expr::eval_or_literal;
use pii_graph::node::{GraphNode, NodeFactory};

/// Config: `{ worker: String, op: String, args: Value }` where `args` may
/// contain `$.`-expressions resolved against the node's eval context.
pub struct WorkerNode {
    pool: Arc<parking_lot::Mutex<std::collections::BTreeMap<String, pii_worker::Supervisor>>>,
    worker: String,
    op: String,
    args: serde_json::Value,
}

impl GraphNode for WorkerNode {
    fn run(&self, ctx: &Ctx, input: &NodeInput) -> Result<Outcome, PiiError> {
        let ec = serde_json::json!({
            "input": input.input,
            "payload": input.payload,
            "nodes": input.nodes,
        });
        let args = resolve(&ec, &self.args)?;
        let sup = self.pool.lock().get(&self.worker).cloned().ok_or_else(|| {
            PiiError::NotFound(format!("worker '{}' is not configured", self.worker))
        })?;
        // F3.7: a dead/in-flight failure is Unavailable and must reach the
        // scheduler unchanged so RetryPolicy sees is_retryable()==true.
        match sup.invoke(ctx, &self.op, args, 120_000) {
            Ok(v) => Ok(Outcome::Next {
                port: PortName::from("ok"),
                value: v,
            }),
            Err(e) => Err(e),
        }
    }
}

/// Factory carrying the shared worker pool (registered in both the engine
/// and lint registries; the daemon fills the pool from its config).
pub struct WorkerFactory {
    /// The supervisor pool (empty outside the daemon).
    pub pool: Arc<parking_lot::Mutex<std::collections::BTreeMap<String, pii_worker::Supervisor>>>,
}

impl NodeFactory for WorkerFactory {
    fn kind(&self) -> &'static str {
        "worker"
    }

    fn effects(&self) -> Effects {
        Effects::MUTATES_EXTERNAL
    }

    fn required_caps(&self) -> CapKind {
        CapKind::SPAWN
    }

    fn ports(&self, _cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError> {
        Ok(vec![PortName::from("ok"), PortName::from("err")])
    }

    fn build(&self, cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError> {
        let worker = cfg
            .get("worker")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| PiiError::Invalid("worker node requires 'worker' string".into()))?
            .to_owned();
        let op = cfg
            .get("op")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| PiiError::Invalid("worker node requires 'op' string".into()))?
            .to_owned();
        let args = cfg.get("args").cloned().unwrap_or(serde_json::Value::Null);
        Ok(Box::new(WorkerNode {
            pool: Arc::clone(&self.pool),
            worker,
            op,
            args,
        }))
    }
}

/// Recursively resolves `$.`-string leaves in `args` (mirror of the tool node).
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
