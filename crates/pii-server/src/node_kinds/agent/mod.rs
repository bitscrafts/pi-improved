//! `agent` node kind (F2.10): provider loop with tool execution.
//!
//! Phase-1 scope note: the provider/tool loop, retry-once and the
//! `max_turns`→`budget` port are implemented; `assistant_delta` streaming
//! and the compaction gate land with the TUI pass (they need nothing here).
//!
//! Split (2026-09-17) from the single `agent.rs` into `agent/` to stay
//! under the workspace's 450-line gate: `turn.rs` (the `GraphNode::run`
//! turn loop), `tools.rs` (tool execution + live events), `memorize.rs`
//! (MEMORIZE marker + history seeding). This file keeps the node/factory
//! types, model resolution and provider-failure routing.

mod memorize;
mod tests;
mod tools;
mod turn;

use std::sync::Arc;

use pii_abi::{Outcome, PiiError, PortName};
use pii_model::ModelProvider;

use pii_graph::node::GraphNode;

use crate::compaction::Compaction;

/// Agent node config (F2.10).
pub struct AgentNode {
    /// The routing step this node was resolved through (`None` = unrouted;
    /// spec 28 R3).
    pub(crate) route_step: Option<String>,
    pub(crate) system: String,
    pub(crate) max_turns: u32,
    pub(crate) model: Option<String>,
    pub(crate) provider: Arc<dyn ModelProvider>,
    pub(crate) toolset: pii_tools::ToolSet,
    pub(crate) workspace: std::path::PathBuf,
    /// Live-event sender (F12, opus review): emits `ToolCall`/`ToolResult`/
    /// `AssistantDelta` to the bus without touching the journal.
    pub(crate) live: Option<tokio::sync::broadcast::Sender<pii_proto::run::SeqEvent>>,
    /// Compaction knobs (spec 12, R1).
    pub(crate) compaction: Compaction,
    /// Global memory store (spec 09 v2, P4.3): `Some` when the graph opts in
    /// via `memory = { enabled = true }`; `None` = no memory behavior.
    pub(crate) memory: Option<Arc<pii_store::MemStore>>,
}

/// Factory registered into the engine's registry.
pub struct AgentFactory {
    /// Provider all agent nodes share.
    pub provider: Arc<dyn ModelProvider>,
    /// `[evolve.routing]` step -> model policy (spec 28 R1). Empty = no
    /// routing behavior; agent nodes without `route_step` are unchanged.
    pub route_policy: std::collections::BTreeMap<String, String>,
    /// Workspace bound to bash mediation.
    pub workspace: std::path::PathBuf,
    /// Live-event sender (F12).
    pub live: Option<tokio::sync::broadcast::Sender<pii_proto::run::SeqEvent>>,
    /// Global memory store (spec 09 v2, P4.3).
    pub memory: Option<Arc<pii_store::MemStore>>,
}

/// Resolves an agent node's model (spec 28 R2/R4).
///
/// `route_step` present → look the step up in `[evolve.routing]`; a missing
/// step is LOUD (`Err(NotFound)`, never a silent default).
///
/// # Errors
/// [`PiiError::NotFound`] when a routed step has no policy entry.
/// No `route_step` → the existing `model` key (or provider default),
/// unchanged.
/// Test seam: same logic as [`resolve_model`], public so the spec-28 TDD
/// contract's integration tests exercise the exact resolution path.
#[allow(clippy::double_must_use, unused_qualifications)]
// reason:
// the Result error IS the load-bearing R4 surface; the fully-qualified map
// type keeps the test seam self-documenting.
#[must_use]
pub fn resolve_model_for_test(
    cfg: &serde_json::Value,
    policy: &std::collections::BTreeMap<String, String>,
) -> Result<Option<String>, PiiError> {
    resolve_model(cfg, policy)
}

pub(super) fn resolve_model(
    cfg: &serde_json::Value,
    policy: &std::collections::BTreeMap<String, String>,
) -> Result<Option<String>, PiiError> {
    let Some(step) = cfg.get("route_step").and_then(serde_json::Value::as_str) else {
        return Ok(cfg
            .get("model")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned));
    };
    policy.get(step).map_or_else(
        // R4: an unassigned step must fail loudly, not default.
        || {
            Err(PiiError::NotFound(format!(
                "route_step '{step}' has no entry in [evolve.routing] — refusing to silently default"
            )))
        },
        |model| Ok(Some(model.clone())),
    )
}

/// A2/A3 seam: `PiiError::Provider` is classified and routed to the
/// `fail` port; every other error kind keeps the pre-A3 `err` port shape
/// (`{"error": <Display>}`) — this function narrows, it does not widen,
/// what counts as a routable failure.
pub(super) fn route_provider_failure(e: &PiiError) -> Outcome {
    if let PiiError::Provider(pf) = e {
        let reason = pii_abi::classify_provider(pf);
        return Outcome::Next {
            port: PortName::from("fail"),
            value: serde_json::to_value(pii_abi::FailurePayload {
                status: pf.status,
                reason,
                provider: pf.provider.clone(),
            })
            .unwrap_or_else(|_| serde_json::json!({ "error": e.to_string() })),
        };
    }
    Outcome::Next {
        port: PortName::from("err"),
        value: serde_json::json!({ "error": e.to_string() }),
    }
}

impl pii_graph::node::NodeFactory for AgentFactory {
    fn kind(&self) -> &'static str {
        "agent"
    }

    fn effects(&self) -> pii_abi::Effects {
        pii_abi::Effects::MUTATES_EXTERNAL
    }

    fn required_caps(&self) -> pii_core::CapKind {
        pii_core::CapKind::FS_READ | pii_core::CapKind::FS_WRITE | pii_core::CapKind::PROC
    }

    fn ports(&self, _cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError> {
        Ok(vec![
            PortName::from("ok"),
            PortName::from("err"),
            PortName::from("budget"),
            PortName::from("fail"),
        ])
    }

    fn build(&self, cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError> {
        let system = cfg
            .get("system")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("You are pii.")
            .to_owned();
        let tools: Vec<String> = cfg
            .get("tools")
            .and_then(serde_json::Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let base = pii_tools::ToolSet::builtin(self.workspace.clone());
        let toolset = if tools.is_empty() {
            base
        } else {
            base.subset(&tools)
        };
        // G1.1 (spec 09 v2): the agent opts into global memory only when the
        // graph declares `memory = { enabled = true }`. Absent/disabled → None
        // (zero behavior change for non-memory graphs).
        let memory = cfg
            .get("memory")
            .and_then(serde_json::Value::as_object)
            .and_then(|m| m.get("enabled"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
            .then(|| self.memory.clone())
            .flatten();
        let route_step = cfg
            .get("route_step")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        Ok(Box::new(AgentNode {
            route_step,
            system,
            max_turns: cfg
                .get("max_turns")
                .and_then(serde_json::Value::as_u64)
                .and_then(|v| u32::try_from(v).ok())
                .unwrap_or(24),
            model: resolve_model(cfg, &self.route_policy)?,
            provider: self.provider.clone(),
            toolset,
            workspace: self.workspace.clone(),
            live: self.live.clone(),
            compaction: Compaction::default(),
            memory,
        }))
    }
}

// ChatResponse import keeps the scripted fixture contract visible here.
#[allow(dead_code)]
fn _resp_shape(_r: &pii_model::ChatResponse) {}
