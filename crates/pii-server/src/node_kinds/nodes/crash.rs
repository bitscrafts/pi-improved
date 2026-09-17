//! `debug_crash` node kind: a deliberate crash trigger.
//!
//! Registered solely so `scripts/phase3/crash.sh` can exercise the panic
//! hook (F5.4) without the daemon path itself ever needing to panic. It
//! divides by a runtime-only zero (never a compile-time-provable literal),
//! so no `clippy::panic`/`unwrap_used`/`indexing_slicing` lint applies and
//! no new `#[allow]` is needed anywhere.

use pii_abi::{NodeInput, Outcome, PiiError, PortName};
use pii_core::{CapKind, Ctx};

use pii_graph::node::{GraphNode, NodeFactory};

/// Registered into [`crate::registry`] only (not `lint_registry`): this
/// kind exists purely to test F5.4's crash file, not to lint-pass a
/// production graph.
pub struct DebugCrashFactory;

impl NodeFactory for DebugCrashFactory {
    fn kind(&self) -> &'static str {
        "debug_crash"
    }

    fn effects(&self) -> pii_abi::Effects {
        pii_abi::Effects::PURE
    }

    fn required_caps(&self) -> CapKind {
        CapKind::empty()
    }

    fn ports(&self, _cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError> {
        Ok(vec![PortName::from("ok")])
    }

    fn build(&self, _cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError> {
        Ok(Box::new(DebugCrashNode))
    }
}

struct DebugCrashNode;

impl GraphNode for DebugCrashNode {
    fn run(&self, _ctx: &Ctx, input: &NodeInput) -> Result<Outcome, PiiError> {
        let zero = input
            .input
            .get("zero")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let one: u64 = 1;
        // `zero` is a runtime value the compiler cannot const-fold: this is
        // a genuine division-by-zero panic, the whole point of this node
        // (F5.4 crash-hook test seam). `arithmetic_side_effects` is already
        // `allow` at workspace scope, so no lint override is needed here.
        let out = one / zero;
        Ok(Outcome::Next {
            port: PortName::from("ok"),
            value: serde_json::json!(out),
        })
    }
}
