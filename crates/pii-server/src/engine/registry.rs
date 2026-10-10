//! Node registries.
//!
//! Split out of `lib.rs` (2026-09-05) to stay under the workspace's
//! per-file line-count lint: the live daemon registry ([`registry`]),
//! the static-lint-only registry ([`lint_registry`]), and the built-in
//! single-agent-node graph ([`DEFAULT_TOML`], used by `Engine::load_spec`
//! when no `default.toml` file exists).

use std::collections::BTreeMap;
use std::sync::Arc;

use pii_graph::node::NodeRegistry;
use pii_model::ModelProvider;
use pii_tools::ToolSet;

use crate::EngineConfig;
use crate::engine::tool;
use crate::node_kinds::{agent, nodes};

/// Builds the registry with the built-ins plus `agent`, `tool`, `gate` and
/// `memory` (F2.2, F5.9, F6.7).
#[must_use]
#[allow(clippy::too_many_arguments)] // reason: the factory set mirrors the
// Engine owned resources; a config struct would just be re-destructured.
pub fn registry(
    cfg: &EngineConfig,
    provider: Arc<dyn ModelProvider>,
    live: tokio::sync::broadcast::Sender<pii_proto::run::SeqEvent>,
    approvals: Arc<pii_store::approval::ApprovalStore>,
    heals: Arc<pii_store::heal::HealStore>,
    memory: Arc<pii_store::MemStore>,
    workers: Arc<parking_lot::Mutex<BTreeMap<String, pii_worker::Supervisor>>>,
    route_policy: BTreeMap<String, String>,
    spawn_engine: nodes::spawn::SpawnEngineHandle,
    log: Arc<dyn pii_observe::Observer>,
    tiers: Arc<crate::tiers::TierTable>,
) -> NodeRegistry {
    let mut reg = NodeRegistry::builtin();
    reg.register(Arc::new(agent::AgentFactory {
        provider,
        workspace: cfg.workspace.clone(),
        live: Some(live),
        memory: Some(Arc::clone(&memory)),
        route_policy,
        tiers: Some(tiers),
    }));
    reg.register(Arc::new(tool::ToolFactory {
        toolset: ToolSet::builtin(cfg.workspace.clone()),
    }));
    reg.register(Arc::new(nodes::gate::GateFactory { approvals }));
    reg.register(Arc::new(nodes::heal::HealFactory {
        heals: Arc::clone(&heals),
        runner: Arc::new(nodes::heal::CargoXtaskRunner {
            workspace: cfg.workspace.clone(),
        }),
    }));
    reg.register(Arc::new(nodes::heal::HealRecordFactory { heals }));
    reg.register(Arc::new(nodes::sop::SopFactory {
        log: Arc::clone(&log),
    }));
    reg.register(Arc::new(nodes::memory::MemoryFactory {
        store: memory,
        log,
    }));
    reg.register(Arc::new(nodes::crash::DebugCrashFactory));
    reg.register(Arc::new(nodes::worker::WorkerFactory { pool: workers }));
    reg.register(Arc::new(nodes::spawn::SpawnFactory {
        engine: spawn_engine,
    }));
    reg
}

/// Registers `remember`/`recall` over a fresh, empty hub and returns it (spec 126). The daemon
/// fills the hub; a lint registry never does, because lint never executes a node.
pub fn register_entity_nodes(reg: &mut NodeRegistry) -> Arc<pii_memory::entity::EntityHub> {
    let hub = Arc::new(pii_memory::entity::EntityHub::default());
    reg.register(Arc::new(nodes::entity::RememberFactory {
        hub: Arc::clone(&hub),
    }));
    reg.register(Arc::new(nodes::entity::RecallFactory {
        hub: Arc::clone(&hub),
    }));
    hub
}

/// A registry for static linting only.
///
/// Registers `agent`/`tool`/`gate`/`memory` so `pii graph lint` accepts
/// starter graphs, with a provider that is never called and scratch stores
/// that are never meaningfully written (lint never executes a node).
///
/// **B3 (opus review)**: the lint scratch DBs used to be hardcoded to a
/// literal `_tmp/phase3/lint-{approval,memory}.redb` path under one specific
/// checkout root, so on any other machine (or a different runtime dir) both
/// opens failed and `gate`/`memory` were silently dropped from the registry —
/// `pii graph lint` then reported a false `V0: unknown kind 'memory'` for
/// graphs that are actually fine. They are now placed under `runtime_dir`
/// (the same directory the caller's real store/config already lives in),
/// which is always a real, writable path in this process, not a literal
/// string that only exists in this checkout.
#[must_use]
pub fn lint_registry(workspace: &std::path::Path, runtime_dir: &std::path::Path) -> NodeRegistry {
    lint_registry_with_log(workspace, runtime_dir, Arc::new(pii_observe::NullObserver))
}

/// [`lint_registry`] whose `sop` nodes report to `log` — what `pii sop run`
/// uses so a CLI run emits the `sop.step.*` events somewhere visible.
#[must_use]
pub fn lint_registry_with_log(
    workspace: &std::path::Path,
    runtime_dir: &std::path::Path,
    log: Arc<dyn pii_observe::Observer>,
) -> NodeRegistry {
    let mut reg = NodeRegistry::builtin();
    reg.register(Arc::new(nodes::mqtt::MqttFactory {
        hub: Arc::new(pii_channels::mqtt::MqttHub::default()),
    }));
    reg.register(Arc::new(nodes::hal::HalFactory {
        hub: Arc::new(pii_hal::HalHub::default()),
        log: Arc::clone(&log),
    }));
    reg.register(Arc::new(nodes::wasm::WasmFactory {
        hub: Arc::new(pii_wasm::WasmHub::default()),
        log: Arc::clone(&log),
    }));
    drop(register_entity_nodes(&mut reg));
    reg.register(Arc::new(nodes::sop::SopFactory { log }));
    reg.register(Arc::new(nodes::worker::WorkerFactory {
        pool: Arc::new(parking_lot::Mutex::new(BTreeMap::<
            String,
            pii_worker::Supervisor,
        >::new())),
    }));
    reg.register(Arc::new(agent::AgentFactory {
        provider: Arc::new(pii_model::ScriptedProvider::empty()),
        workspace: workspace.to_path_buf(),
        live: None,
        memory: None,
        route_policy: BTreeMap::new(),
        tiers: None,
    }));
    reg.register(Arc::new(tool::ToolFactory {
        toolset: ToolSet::builtin(workspace.to_path_buf()),
    }));
    // B2 (opus review): these opens are best-effort (lint must still work
    // when the runtime dir isn't writable), but a failure here means
    // `gate`/`memory` graphs will fail lint with `unknown kind` rather than
    // silently validating against an empty capset — surfaced via the
    // eprintln below instead of vanishing.
    match pii_store::approval::ApprovalStore::open(&runtime_dir.join("lint-approval.redb")) {
        Ok(approvals) => reg.register(Arc::new(nodes::gate::GateFactory {
            approvals: Arc::new(approvals),
        })),
        Err(e) => {
            #[allow(clippy::print_stderr)]
            {
                eprintln!(
                    "lint_registry: gate unavailable (approval store: {e}); 'gate' nodes will fail lint as unknown kind"
                );
            }
        }
    }
    match pii_store::MemStore::open(&runtime_dir.join("lint-memory.redb")) {
        Ok(memory) => reg.register(Arc::new(nodes::memory::MemoryFactory {
            store: Arc::new(memory),
            log: Arc::new(pii_observe::NullObserver),
        })),
        Err(e) => {
            #[allow(clippy::print_stderr)]
            {
                eprintln!(
                    "lint_registry: memory unavailable (mem store: {e}); 'memory' nodes will fail lint as unknown kind"
                );
            }
        }
    }
    // S0.2: `heal_detect` is a server-owned kind like `gate`/`memory`; the
    // lint registry registers it with a cargo-xtask runner so
    // `graphs/starter/heal-watch.toml` lints (lint never executes nodes).
    match pii_store::heal::HealStore::open(&runtime_dir.join("lint-heal.redb")) {
        Ok(heals) => {
            let heals = Arc::new(heals);
            reg.register(Arc::new(nodes::heal::HealFactory {
                heals: Arc::clone(&heals),
                runner: Arc::new(nodes::heal::CargoXtaskRunner {
                    workspace: workspace.to_path_buf(),
                }),
            }));
            reg.register(Arc::new(nodes::heal::HealRecordFactory { heals }));
        }
        Err(e) => {
            #[allow(clippy::print_stderr)]
            {
                eprintln!(
                    "lint_registry: heal_detect unavailable (heal store: {e}); 'heal_detect' nodes will fail lint as unknown kind"
                );
            }
        }
    }
    reg
}

/// The built-in session graph: one `agent` node wired `ok → done`. Both
/// `caps` lines below include `net` (2026-09-05) - node caps default to
/// `required_caps()`, which excludes NET by design, so the graph-level
/// line alone is not enough.
pub(crate) const DEFAULT_TOML: &str = r#"
name = "default"
entry = "agent"
caps = ["fs_read", "fs_write", "proc", "net"]

# Spec 12, R2: the built-in session graph declares a token budget so the
# agent node's per-turn charge (B6) has a real limit to enforce instead of
# `u64::MAX`.
[limits]
max_tokens = 200_000

[[node]]
id = "agent"
kind = "agent"
caps = ["fs_read", "fs_write", "proc", "net"]
system = "You are pii, a coding agent. Answer concisely."

[[edge]]
from = "agent"
port = "ok"
to = "done"
kind = "data"

[[node]]
id = "done"
kind = "transform"
expr = "$.payload"
"#;

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    /// Spec 12, R2: the built-in session graph carries a `[limits]`
    /// `max_tokens` so the agent node's budget charge has a real ceiling.
    #[test]
    fn default_toml_has_limits() {
        let doc: toml::Table = toml::from_str(super::DEFAULT_TOML).expect("parse DEFAULT_TOML");
        let max_tokens = doc
            .get("limits")
            .and_then(|l| l.get("max_tokens"))
            .and_then(toml::Value::as_integer)
            .expect("[limits] max_tokens present");
        assert!(max_tokens > 0, "max_tokens must be a positive bound");
    }
}
