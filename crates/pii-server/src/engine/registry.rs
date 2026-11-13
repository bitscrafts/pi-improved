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
    podman: std::path::PathBuf,
) -> NodeRegistry {
    let mut reg = NodeRegistry::builtin();
    if let Some(ceiling) = cfg.grant_ceiling {
        reg = reg.with_grant_ceiling(ceiling);
    }
    // Spec 363 R1: the planner role calls the same provider the agent does; its node names the model.
    reg.register(Arc::new(nodes::plan::PlanFactory {
        provider: Arc::clone(&provider),
    }));
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
        heals,
        runner: Arc::new(nodes::heal::VerdictRunner {
            workspace: cfg.workspace.clone(),
        }),
    }));
    reg.register(Arc::new(nodes::sop::SopFactory {
        log: Arc::clone(&log),
    }));
    // Spec 227 R2: the coder's pinned skills come from the same store the memory nodes use, and each run's skill
    // files are written beside it, outside the project.
    let store_dir = cfg
        .store_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    reg.register(Arc::new(
        nodes::coder::CoderFactory::new(Arc::clone(&log))
            .with_podman(podman)
            .with_skills(Arc::new(nodes::coder_skills::StoreSkills::new(Arc::clone(
                &memory,
            ))))
            .with_models(Arc::new(cfg.models.clone()))
            .with_runs_dir(store_dir.join("runs")),
    ));
    reg.register(Arc::new(nodes::memory::MemoryFactory {
        store: memory,
        log,
    }));
    reg.register(Arc::new(nodes::crash::DebugCrashFactory));
    reg.register(Arc::new(nodes::classify::ClassifyFactory));
    reg.register(Arc::new(nodes::worker::WorkerFactory { pool: workers }));
    reg.register(Arc::new(nodes::spawn::SpawnFactory {
        engine: spawn_engine,
    }));
    reg
}

/// Registers the queue graph's `judge` and `ledger` kinds over `store` and `heals` (spec 208, spec 391).
///
/// The store is the one source of truth for the development queue; the nodes never read `plan.tsv`. The four retired
/// kinds are registered too, so a graph that still names one refuses at load.
pub fn register_queue_nodes(
    reg: &mut NodeRegistry,
    store: Arc<pii_store::queue::QueueStore>,
    memory: Option<Arc<pii_store::MemStore>>,
    heals: Option<Arc<pii_store::heal::HealStore>>,
    log: Option<Arc<dyn pii_observe::Observer>>,
    workspace: &std::path::Path,
) {
    reg.register(Arc::new(nodes::queue::JudgeFactory));
    reg.register(Arc::new(nodes::ledger::WorkspaceLedgerFactory {
        ledger: nodes::ledger::LedgerFactory {
            queue: store,
            memory,
            heals,
            gates: Some(Arc::new(nodes::heal::VerdictRunner {
                workspace: workspace.to_path_buf(),
            })),
            log,
        },
        workspace: workspace.to_path_buf(),
    }));
    for retired in nodes::ledger::retired_kinds() {
        reg.register(retired);
    }
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

/// Registers `post` over a fresh hub and returns it (spec 128); the daemon fills the hub.
pub fn register_post_node(
    reg: &mut NodeRegistry,
    log: Arc<dyn pii_observe::Observer>,
) -> Arc<pii_channels::post::ChannelHub> {
    let hub = Arc::new(pii_channels::post::ChannelHub::default());
    reg.register(Arc::new(nodes::post::PostFactory {
        hub: Arc::clone(&hub),
        log,
    }));
    hub
}

/// The channel-facing kinds for the lint registry: empty hubs, because lint never runs a node.
fn register_lint_channel_kinds(reg: &mut NodeRegistry, log: &Arc<dyn pii_observe::Observer>) {
    reg.register(Arc::new(nodes::mqtt::MqttFactory {
        hub: Arc::new(pii_channels::mqtt::MqttHub::default()),
    }));
    reg.register(Arc::new(nodes::classify::ClassifyFactory));
    reg.register(Arc::new(nodes::post::PostFactory {
        hub: Arc::new(pii_channels::post::ChannelHub::default()),
        log: Arc::clone(log),
    }));
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
/// graphs that are actually fine.
///
/// Spec 371 R1: the scratch stores are in memory (`redb::backends::InMemoryBackend`,
/// opened by `pii_store`), so a lint leaves no `lint-*.redb` file in any directory.
/// `runtime_dir` is kept for callers and is no longer written to.
#[must_use]
pub fn lint_registry(workspace: &std::path::Path, runtime_dir: &std::path::Path) -> NodeRegistry {
    lint_registry_with_log(
        workspace,
        runtime_dir,
        Arc::new(pii_observe::StderrObserver),
    )
}

/// The lint registry's queue kinds, over an in-memory lint-only store (spec 208, spec 371 R1).
fn register_lint_queue_nodes(
    reg: &mut NodeRegistry,
    log: &Arc<dyn pii_observe::Observer>,
    workspace: &std::path::Path,
) {
    match pii_store::queue::QueueStore::open_in_memory() {
        Ok(queue) => {
            let heals = pii_store::heal::HealStore::open_in_memory()
                .ok()
                .map(Arc::new);
            register_queue_nodes(reg, Arc::new(queue), None, heals, None, workspace);
        }
        Err(e) => store_unavailable(log, "queue", &e),
    }
}

/// Reports a lint store that did not open: its kinds fail lint as unknown (spec 227 R7).
fn store_unavailable(
    log: &Arc<dyn pii_observe::Observer>,
    store: &'static str,
    err: &impl std::fmt::Display,
) {
    log.event(
        &pii_observe::Event::new(
            "lint",
            pii_observe::Level::Warn,
            pii_observe::names::LINT_STORE_UNAVAILABLE,
        )
        .field("store", store)
        .field("reason", err.to_string()),
    );
}

/// [`lint_registry`] whose `sop` nodes report to `log` — what `pii sop run`
/// uses so a CLI run emits the `sop.step.*` events somewhere visible.
#[must_use]
pub fn lint_registry_with_log(
    workspace: &std::path::Path,
    _runtime_dir: &std::path::Path,
    log: Arc<dyn pii_observe::Observer>,
) -> NodeRegistry {
    let mut reg = NodeRegistry::builtin();
    register_lint_channel_kinds(&mut reg, &log);
    reg.register(Arc::new(nodes::hal::HalFactory {
        hub: Arc::new(pii_hal::HalHub::default()),
        log: Arc::clone(&log),
    }));
    reg.register(Arc::new(nodes::wasm::WasmFactory {
        hub: Arc::new(pii_wasm::WasmHub::default()),
        log: Arc::clone(&log),
    }));
    drop(register_entity_nodes(&mut reg));
    // Spec 227 R2: lint never reads the store, so the coder keeps `NoSkills` here; only the daemon's registry
    // supplies the store.
    reg.register(Arc::new(nodes::coder::CoderFactory::new(Arc::clone(&log))));
    reg.register(Arc::new(nodes::worker::WorkerFactory {
        pool: Arc::new(parking_lot::Mutex::new(BTreeMap::<
            String,
            pii_worker::Supervisor,
        >::new())),
    }));
    reg.register(Arc::new(nodes::plan::PlanFactory {
        provider: Arc::new(pii_model::ScriptedProvider::empty()),
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
    // B2 (opus review): these opens are best-effort, but a failure here means
    // `gate`/`memory` graphs will fail lint with `unknown kind` rather than
    // silently validating against an empty capset — surfaced as a
    // `lint.store.unavailable` event instead of vanishing. Spec 371 R1: they
    // are in-memory stores (`InMemoryBackend`), so lint writes no file.
    match pii_store::approval::ApprovalStore::open_in_memory() {
        Ok(approvals) => reg.register(Arc::new(nodes::gate::GateFactory {
            approvals: Arc::new(approvals),
        })),
        Err(e) => store_unavailable(&log, "gate", &e),
    }
    match pii_store::MemStore::open_in_memory() {
        Ok(memory) => reg.register(Arc::new(nodes::memory::MemoryFactory {
            store: Arc::new(memory),
            log: Arc::new(pii_observe::NullObserver),
        })),
        Err(e) => store_unavailable(&log, "memory", &e),
    }
    register_lint_queue_nodes(&mut reg, &log, workspace);
    // S0.2: `heal_detect` is a server-owned kind like `gate`/`memory`; the
    // lint registry registers it with a cargo-xtask runner so
    // `graphs/starter/heal-watch.toml` lints (lint never executes nodes).
    match pii_store::heal::HealStore::open_in_memory() {
        Ok(heals) => {
            let heals = Arc::new(heals);
            reg.register(Arc::new(nodes::heal::HealFactory {
                heals,
                runner: Arc::new(nodes::heal::VerdictRunner {
                    workspace: workspace.to_path_buf(),
                }),
            }));
        }
        Err(e) => store_unavailable(&log, "heal_detect", &e),
    }
    reg.register(Arc::new(nodes::sop::SopFactory { log }));
    reg.register(Arc::new(nodes::crash::DebugCrashFactory));
    // Lint never executes a node, so the spawn handle stays empty here, as the daemon's does until attach.
    reg.register(Arc::new(nodes::spawn::SpawnFactory {
        engine: Arc::new(parking_lot::Mutex::new(None)),
    }));
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
