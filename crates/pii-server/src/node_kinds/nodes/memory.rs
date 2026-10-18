//! `memory` node kind (F6.7): `op = "consolidate"` drives
//! [`pii_memory::consolidate::consolidate`]. Zero model calls — the whole
//! crate makes none, no exceptions (§5.4).

use std::sync::Arc;

use pii_abi::{NodeInput, Outcome, PiiError, PortName};
use pii_core::{CapKind, Ctx};
use pii_memory::consolidate::{ConsolidateCfg, ConsolidateReport};
use pii_memory::consolidate_observed;
use pii_store::MemStore;

use pii_graph::node::{GraphNode, NodeFactory};

/// Registered into [`crate::engine::registry`]; every `memory` node shares one
/// memory store.
pub struct MemoryFactory {
    /// The shared memory store.
    pub store: Arc<MemStore>,
    /// Receives the `memory.consolidate` event (spec 108).
    pub log: Arc<dyn pii_observe::Observer>,
}

impl NodeFactory for MemoryFactory {
    fn kind(&self) -> &'static str {
        "memory"
    }

    fn is_effect(&self) -> bool {
        true
    }

    fn effects(&self) -> pii_abi::Effects {
        pii_abi::Effects::MUTATES_EXTERNAL
    }

    fn required_caps(&self) -> CapKind {
        CapKind::empty()
    }

    fn ports(&self, _cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError> {
        Ok(vec![PortName::from("ok"), PortName::from("err")])
    }

    fn build(&self, cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError> {
        let op = cfg
            .get("op")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| PiiError::Invalid(String::from("memory requires 'op' (string)")))?;
        if op != "consolidate" {
            return Err(PiiError::Invalid(format!(
                "memory: unsupported op '{op}' (only 'consolidate' ships this phase)"
            )));
        }
        Ok(Box::new(ConsolidateNode {
            store: Arc::clone(&self.store),
            log: Arc::clone(&self.log),
        }))
    }
}

struct ConsolidateNode {
    store: Arc<MemStore>,
    log: Arc<dyn pii_observe::Observer>,
}

impl GraphNode for ConsolidateNode {
    fn run(&self, ctx: &Ctx, _input: &NodeInput) -> Result<Outcome, PiiError> {
        let cfg = ConsolidateCfg {
            archive_after_days: 90.0,
            same_layer_only: true,
        };
        match consolidate_observed(&self.store, ctx, &cfg, self.log.as_ref()) {
            Ok(report) => Ok(Outcome::Next {
                port: PortName::from("ok"),
                value: report_json(&report),
            }),
            Err(e) => Ok(Outcome::Next {
                port: PortName::from("err"),
                value: serde_json::json!({ "error": e.to_string() }),
            }),
        }
    }
}

fn report_json(r: &ConsolidateReport) -> serde_json::Value {
    serde_json::json!({
        "candidates_evaluated": r.candidates_evaluated,
        "candidates_capped": r.candidates_capped,
        "merged": r.merged,
        "archived": r.archived,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use pii_core::{Budget, CapSet};
    use pii_observe::{Recorder, names};

    use super::*;

    /// Spec 108: the `memory` node's consolidation reports through the log
    /// the factory was given.
    #[test]
    fn consolidate_node_reports_to_the_log() {
        let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "memnode-").unwrap();
        let store = Arc::new(MemStore::open(&dir.path().join("m.redb")).unwrap());
        let log = Arc::new(Recorder::new());
        let factory = MemoryFactory {
            store,
            log: log.clone(),
        };
        let node = factory
            .build(&serde_json::json!({"op": "consolidate"}))
            .unwrap();
        let ctx = Ctx::root(CapSet::empty(), Budget::unlimited());
        let input = NodeInput {
            input: serde_json::json!({}),
            payload: serde_json::json!({}),
            nodes: serde_json::json!({}),
        };
        node.run(&ctx, &input).unwrap();
        assert_eq!(log.events_named(names::MEMORY_CONSOLIDATE).len(), 1);
    }
}
