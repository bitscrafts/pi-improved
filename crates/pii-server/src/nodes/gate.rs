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
        Ok(Box::new(GateNode {
            policy,
            prompt,
            approvals: Arc::clone(&self.approvals),
        }))
    }
}

struct GateNode {
    policy: String,
    prompt: String,
    approvals: Arc<ApprovalStore>,
}

impl GraphNode for GateNode {
    fn run(&self, ctx: &Ctx, input: &NodeInput) -> Result<Outcome, PiiError> {
        let id = approval_id(ctx);
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

/// The deterministic approval id: `blake3(run_id, node_id)[..16]` hex
/// (16 bytes, 32 hex chars — same convention as `pii_memory::doc::id_of`).
#[must_use]
pub fn approval_id(ctx: &Ctx) -> String {
    use std::fmt::Write as _;
    let (run, node) = match ctx.principal() {
        Principal::GraphNode { run, node } => (run.to_string(), node.to_string()),
        _ => (ctx.trace().run.to_string(), String::new()),
    };
    let digest = blake3::hash(format!("{run}:{node}").as_bytes());
    digest
        .as_bytes()
        .iter()
        .take(16)
        .fold(String::with_capacity(32), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
}
