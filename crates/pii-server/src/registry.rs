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

use crate::{EngineConfig, agent, nodes, tool};

/// Builds the registry with the built-ins plus `agent`, `tool`, `gate` and
/// `memory` (F2.2, F5.9, F6.7).
#[must_use]
pub fn registry(
    cfg: &EngineConfig,
    provider: Arc<dyn ModelProvider>,
    live: tokio::sync::broadcast::Sender<pii_proto::run::SeqEvent>,
    approvals: Arc<pii_store::approval::ApprovalStore>,
    heals: Arc<pii_store::heal::HealStore>,
    memory: Arc<pii_store::MemStore>,
    workers: Arc<parking_lot::Mutex<BTreeMap<String, pii_worker::Supervisor>>>,
) -> NodeRegistry {
    let mut reg = NodeRegistry::builtin();
    reg.register(Arc::new(agent::AgentFactory {
        provider,
        workspace: cfg.workspace.clone(),
        live: Some(live),
    }));
    reg.register(Arc::new(tool::ToolFactory {
        toolset: ToolSet::builtin(cfg.workspace.clone()),
    }));
    reg.register(Arc::new(nodes::gate::GateFactory { approvals }));
    reg.register(Arc::new(nodes::heal::HealFactory {
        heals,
        runner: Arc::new(nodes::heal::CargoXtaskRunner {
            workspace: cfg.workspace.clone(),
        }),
    }));
    reg.register(Arc::new(nodes::memory::MemoryFactory { store: memory }));
    reg.register(Arc::new(nodes::crash::DebugCrashFactory));
    reg.register(Arc::new(nodes::worker::WorkerFactory { pool: workers }));
    reg
}

/// A registry for static linting only.
///
/// Registers `agent`/`tool`/`gate`/`memory` so `pii graph lint` accepts
/// starter graphs, with a provider that is never called and scratch stores
/// that are never meaningfully written (lint never executes a node).
///
/// **B3 (opus review)**: the lint scratch DBs used to be hardcoded to
/// `/projects/pi-improved/_tmp/phase3/lint-{approval,memory}.redb`, so on
/// any other machine (or a different runtime dir) both opens failed and
/// `gate`/`memory` were silently dropped from the registry — `pii graph
/// lint` then reported a false `V0: unknown kind 'memory'` for graphs that
/// are actually fine. They are now placed under `runtime_dir` (the same
/// directory the caller's real store/config already lives in), which is
/// always a real, writable path in this process, not a literal string that
/// only exists in this checkout.
#[must_use]
pub fn lint_registry(workspace: &std::path::Path, runtime_dir: &std::path::Path) -> NodeRegistry {
    let mut reg = NodeRegistry::builtin();
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
        Ok(heals) => reg.register(Arc::new(nodes::heal::HealFactory {
            heals: Arc::new(heals),
            runner: Arc::new(nodes::heal::CargoXtaskRunner {
                workspace: workspace.to_path_buf(),
            }),
        })),
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
