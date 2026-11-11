//! The `refuse` node kind (spec 212, 364, 410). Split out of `nodes.rs` so that file stays under the line rule; `nodes::RefuseNode` is re-exported.

use crate::nodes::eval_ctx;
use crate::template::render;
use pii_abi::{NodeInput, Outcome, PiiError, PortName};
use pii_core::Ctx;

/// `refuse` (spec 212): fails the run by name.
///
/// `hook` is stamped by the weave from the attach id; `reason` is declared by the
/// fragment, and may name `{{path}}` placeholders that are rendered at run time (spec 410). `returns` is set by the load
/// when a host wires this refusal (spec 364): the verdict goes back to the host on `ok`, under that port's name, instead
/// of failing the run.
pub struct RefuseNode {
    hook: String,
    reason: String,
    returns: Option<String>,
}

impl RefuseNode {
    /// Builds from cfg; both fields default to a placeholder rather than failing.
    ///
    /// # Errors
    /// Never fails today; the `Result` keeps the factory signature uniform.
    pub fn new(cfg: &serde_json::Value) -> Result<Self, PiiError> {
        let field = |key: &str, default: &str| {
            cfg.get(key)
                .and_then(|v| v.as_str())
                .unwrap_or(default)
                .to_owned()
        };
        Ok(Self {
            hook: field("hook", "unnamed"),
            reason: field("reason", "refused"),
            returns: cfg
                .get("returns")
                .and_then(|v| v.as_str())
                .map(str::to_owned),
        })
    }
}

impl crate::GraphNode for RefuseNode {
    fn run(&self, _ctx: &Ctx, input: &NodeInput) -> Result<Outcome, PiiError> {
        // Spec 410: a placeholder is read from this node's own context. A reason that cannot be rendered is used as
        // written (R2): the refusal still refuses, and its error must not hide the failure it reports.
        let reason = render(&eval_ctx(input), &self.reason).unwrap_or_else(|_| self.reason.clone());
        // A join's payload is keyed by source, and the arm that arrived says why.
        let arm = input
            .payload
            .as_object()
            .and_then(|arms| arms.values().find(|v| v.get("reason").is_some()))
            .unwrap_or(&input.payload);
        let why = arm.get("reason").and_then(|v| v.as_str());
        let Some(port) = &self.returns else {
            // The configured text is a fallback for a refusal that arrives with no reason (a coder that
            // really failed). When the verdict that reached this node CARRIES one, it is the answer:
            // `sdd`'s contract judge refuses a row whose criteria already pass, and the run used to fail
            // as "the agent failed" for rows that never reached a coder (measured 2026-11-05: eight
            // dispatches recorded as worker failures, none of them ever spent).
            let text = why.map_or_else(|| reason.clone(), |why| format!("{reason}: {why}"));
            return Ok(Outcome::Fail {
                error: format!("run.refused{{hook={}, reason={text}}}", self.hook),
            });
        };
        let text = why.map_or_else(
            || format!("{port}: {reason}"),
            |why| format!("{port}: {reason}: {why}"),
        );
        let criterion = arm.pointer("/failing/0").cloned();
        Ok(Outcome::Next {
            port: PortName::from("ok"),
            value: serde_json::json!({
                "verdict": "refused",
                "port": port,
                "reason": text,
                "criterion": criterion,
            }),
        })
    }
}
