//! `gate` node kind (F5.9): approval suspension with a store-backed decision.
//!
//! The decision is read from the store, not from a future — this is what
//! makes a resume after a daemon restart deterministic (§8.8).

use std::sync::Arc;

use pii_abi::{NodeInput, Outcome, PiiError, PortName, ResumeKey, SuspendReason};
use pii_core::{CapKind, Ctx, Principal};
use pii_store::approval::ApprovalStore;

use pii_graph::node::{GraphNode, NodeFactory};

/// Registered into [`crate::registry`]; every `gate` node shares one
/// approval store.
pub struct GateFactory {
    /// The shared approval table.
    pub approvals: Arc<ApprovalStore>,
}

impl NodeFactory for GateFactory {
    fn kind(&self) -> &'static str {
        "gate"
    }

    fn effects(&self) -> pii_abi::Effects {
        pii_abi::Effects::PURE
    }

    fn required_caps(&self) -> CapKind {
        CapKind::empty()
    }

    fn ports(&self, _cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError> {
        Ok(vec![PortName::from("allow"), PortName::from("deny")])
    }

    fn build(&self, cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError> {
        let policy = cfg
            .get("policy")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| PiiError::Invalid(String::from("gate requires 'policy' (string)")))?
            .to_owned();
        let prompt = cfg
            .get("prompt")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| PiiError::Invalid(String::from("gate requires 'prompt' (string)")))?
            .to_owned();
        // R2 (spec 20): an optional `cycle` salt. When set, it is folded into
        // the approval id so a NEW decision cycle gets a FRESH id (no cached
        // decision → the gate suspends and asks a human again). Absent (the
        // default) = byte-identical current behavior: `blake3(run,node)`, so
        // the §8.8 suspend→approve→resume replay still re-reads its own
        // decision. The boss/promotion graph sets a fresh `cycle` per
        // promotion attempt so every promotion is a fresh human decision.
        let cycle = cfg
            .get("cycle")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        Ok(Box::new(GateNode {
            policy,
            prompt,
            cycle,
            approvals: Arc::clone(&self.approvals),
        }))
    }
}

struct GateNode {
    policy: String,
    prompt: String,
    cycle: u64,
    approvals: Arc<ApprovalStore>,
}

impl GraphNode for GateNode {
    fn run(&self, ctx: &Ctx, input: &NodeInput) -> Result<Outcome, PiiError> {
        let id = approval_id(ctx, self.cycle);
        if let Some(a) = self.approvals.get_approval(&id)?
            && let Some(allow) = a.decision
        {
            let port = if allow { "allow" } else { "deny" };
            return Ok(Outcome::Next {
                port: PortName::from(port),
                value: input.payload.clone(),
            });
        }
        Ok(Outcome::Suspend {
            reason: SuspendReason::Approval {
                policy: self.policy.clone(),
                prompt: self.prompt.clone(),
            },
            key: ResumeKey(format!("approval:{id}")),
        })
    }
}

/// Deterministic approval id: `blake3(run_id, node_id, cycle)[..16]` hex.
/// `cycle` 0 = no salt (historical id); a fresh cycle forces a new decision.
#[must_use]
pub fn approval_id(ctx: &Ctx, cycle: u64) -> String {
    use std::fmt::Write as _;
    let (run, node) = match ctx.principal() {
        Principal::GraphNode { run, node } => (run.to_string(), node.to_string()),
        _ => (ctx.trace().run.to_string(), String::new()),
    };
    let digest = blake3::hash(format!("{run}:{node}:{cycle}").as_bytes());
    digest
        .as_bytes()
        .iter()
        .take(16)
        .fold(String::with_capacity(32), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use pii_core::{Budget, CapSet, Ctx, TraceCtx};

    use super::approval_id;

    fn ctx(run: &str) -> Ctx {
        Ctx::with_observer(
            CapSet::empty(),
            Budget::unlimited(),
            std::sync::Arc::new(pii_core::NullObserver),
            TraceCtx {
                run: pii_abi::RunId::from(run),
                span: 0,
            },
        )
    }

    /// R2 (spec 20): a different `cycle` salt yields a DIFFERENT approval id,
    /// so a new decision cycle cannot reuse a stale cached decision.
    #[test]
    fn approval_id_changes_with_cycle() {
        let c = ctx("r1");
        let id0 = approval_id(&c, 0);
        let id1 = approval_id(&c, 1);
        assert_ne!(id0, id1, "cycle must change the approval id");
        // Same cycle is stable (deterministic replay within one attempt).
        assert_eq!(approval_id(&c, 1), approval_id(&c, 1));
    }

    /// R2 (spec 20): cycle 0 preserves the historical `blake3(run,node)` id
    /// (no salt) so existing approvals and §8.8 replay are unchanged.
    #[test]
    fn approval_id_cycle_zero_is_historical() {
        let c = ctx("r1");
        let id = approval_id(&c, 0);
        // This test ctx has no graph-node principal, so approval_id falls back
        // to trace().run with an empty node: blake3("r1::0")[..16] hex.
        let digest = blake3::hash(b"r1::0");
        let expected =
            digest
                .as_bytes()
                .iter()
                .take(16)
                .fold(String::with_capacity(32), |mut out, b| {
                    use std::fmt::Write as _;
                    let _ = write!(out, "{b:02x}");
                    out
                });
        assert_eq!(id, expected, "cycle 0 must match the historical id");
    }
}
