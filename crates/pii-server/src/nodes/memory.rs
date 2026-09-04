//! `memory` node kind (F6.7): `op = "consolidate"` drives
//! [`pii_memory::consolidate::consolidate`]. Zero model calls — the whole
//! crate makes none, no exceptions (§5.4).

use std::sync::Arc;

use pii_abi::{NodeInput, Outcome, PiiError, PortName};
use pii_core::{CapKind, Ctx};
use pii_memory::consolidate::{ConsolidateCfg, ConsolidateReport, consolidate};
use pii_store::MemStore;

use pii_graph::node::{GraphNode, NodeFactory};

/// Registered into [`crate::registry`]; every `memory` node shares one
/// memory store.
pub struct MemoryFactory {
    /// The shared memory store.
    pub store: Arc<MemStore>,
}

impl NodeFactory for MemoryFactory {
    fn kind(&self) -> &'static str {
        "memory"
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
        }))
    }
}

struct ConsolidateNode {
    store: Arc<MemStore>,
}

impl GraphNode for ConsolidateNode {
    fn run(&self, ctx: &Ctx, _input: &NodeInput) -> Result<Outcome, PiiError> {
        let cfg = ConsolidateCfg {
            archive_after_days: 90.0,
            same_layer_only: true,
        };
        match consolidate(&self.store, ctx, &cfg) {
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
