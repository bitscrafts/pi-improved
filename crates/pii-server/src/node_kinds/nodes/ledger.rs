//! The `ledger` node kind (spec 391): one kind for the writes that change stored state other runs read.
//!
//! `op` picks the write: `queue.claim` claims a queue row, `queue.set_status` writes its verdict, `queue.put` adds a row
//! and `heal.record_run` writes a heal attempt. The kind requires the `LEDGER` capability, so a graph must be granted it.
//! The four kinds it replaces refuse at load and name the op that replaces them.

use std::sync::Arc;

use pii_abi::{Effects, PiiError, PortName};
use pii_core::CapKind;
use pii_graph::node::{GraphNode, NodeFactory};
use pii_store::MemStore;
use pii_store::heal::HealStore;
use pii_store::queue::QueueStore;

use super::enqueue::EnqueueNode;
use super::heal;
use super::peer::peer_of;
use super::queue::{RecordNode, SelectNode};

/// The four ops, each with the kind it retires (spec 391 R1).
const OPS: [(&str, &str); 4] = [
    ("queue.claim", "select"),
    ("queue.set_status", "record"),
    ("queue.put", "enqueue"),
    ("heal.record_run", "heal_record"),
];

/// The op of a `ledger` node, which must be one of [`OPS`] (spec 391 R3).
fn op_of(cfg: &serde_json::Value) -> Result<&str, PiiError> {
    let op = cfg
        .get("op")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if OPS.iter().any(|(name, _)| *name == op) {
        return Ok(op);
    }
    let known: Vec<&str> = OPS.iter().map(|(name, _)| *name).collect();
    Err(PiiError::Invalid(format!(
        "ledger op '{op}' is not one of {}",
        known.join(", ")
    )))
}

/// Factory for `ledger`: one kind over the queue store and the heal ledger, one arm per `op`.
pub struct LedgerFactory {
    /// The queue store the claim, the verdict and the row are written to.
    pub queue: Arc<QueueStore>,
    /// The memory a verdict is learned into; `None` in lint, where nothing is written.
    pub memory: Option<Arc<MemStore>>,
    /// The heal-attempt ledger `heal.record_run` writes; `None` where no heal ledger is open.
    pub heals: Option<Arc<HealStore>>,
}

impl NodeFactory for LedgerFactory {
    fn kind(&self) -> &'static str {
        "ledger"
    }

    fn is_effect(&self) -> bool {
        true
    }

    /// A heal attempt was never an effect (its replay is not refused), so its op keeps that class (spec 391 R4).
    fn is_effect_for(&self, cfg: &serde_json::Value) -> bool {
        cfg.get("op").and_then(serde_json::Value::as_str) != Some("heal.record_run")
    }

    fn effects(&self) -> Effects {
        Effects::MUTATES_EXTERNAL
    }

    fn required_caps(&self) -> CapKind {
        CapKind::LEDGER
    }

    fn ports(&self, cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError> {
        let names: &[&str] = match op_of(cfg)? {
            "queue.claim" => &["ok", "blocked", "empty", "running"],
            "queue.put" => &["ok", "err"],
            _ => &["ok"],
        };
        Ok(names.iter().copied().map(PortName::from).collect())
    }

    fn build(&self, cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError> {
        let queue = Arc::clone(&self.queue);
        let node: Box<dyn GraphNode> = match op_of(cfg)? {
            "queue.claim" => Box::new(SelectNode { store: queue }),
            "queue.set_status" => Box::new(RecordNode {
                select: peer_of(cfg, "select"),
                store: queue,
                memory: self.memory.clone(),
            }),
            "queue.put" => Box::new(EnqueueNode {
                store: queue,
                cfg: cfg.clone(),
            }),
            _ => {
                let heals = self.heals.as_ref().ok_or_else(|| {
                    PiiError::Invalid(
                        "ledger op 'heal.record_run' needs the heal ledger".to_owned(),
                    )
                })?;
                heal::record_run(heals, cfg)?
            }
        };
        Ok(node)
    }
}

/// A retired kind (spec 391 R1): it refuses at load and names the `ledger` op that replaces it. It holds no capability,
/// so a graph that still names one cannot reach the store by the old spelling.
struct RetiredKind {
    kind: &'static str,
    op: &'static str,
}

impl RetiredKind {
    fn refusal(&self) -> PiiError {
        PiiError::Invalid(format!(
            "kind '{}' is retired: write kind = \"ledger\" with op = \"{}\"",
            self.kind, self.op
        ))
    }
}

impl NodeFactory for RetiredKind {
    fn kind(&self) -> &'static str {
        self.kind
    }

    fn effects(&self) -> Effects {
        Effects::MUTATES_EXTERNAL
    }

    fn required_caps(&self) -> CapKind {
        CapKind::empty()
    }

    fn ports(&self, _cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError> {
        Err(self.refusal())
    }

    fn build(&self, _cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError> {
        Err(self.refusal())
    }
}

/// The four retired kinds, one per op in [`OPS`].
#[must_use]
pub fn retired_kinds() -> Vec<Arc<dyn NodeFactory>> {
    OPS.iter()
        .map(|&(op, kind)| Arc::new(RetiredKind { kind, op }) as Arc<dyn NodeFactory>)
        .collect()
}
