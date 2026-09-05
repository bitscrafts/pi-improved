//! `pii-server`: the daemon side of the boundary (F2).
//!
//! The engine owns the store, the node registry (built-ins plus `agent`),
//! the model provider, and the tool set. Runs execute on blocking threads;
//! the journal→bus bridge is a [`pii_core::Observer`] so **the Phase-1
//! journaler stays the only redb writer** (the spec instructs reuse of the
//! existing run entry rather than a second journaling path).

pub mod agent;
pub mod bus;
pub mod conn;
pub mod daemon;
pub mod dispatch;
pub mod loopback;
pub mod nodes;
pub mod runs;
pub mod subscribe;
pub mod tool;
pub mod uds;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use pii_abi::{PiiError, RunId};
use pii_graph::node::NodeRegistry;
use pii_graph::spec::GraphSpec;
use pii_model::ModelProvider;
use pii_proto::session::{ContentBlock, SessionStatus, WireMessage};
use pii_tools::ToolSet;
use pii_tools::sandbox::SandboxProfileName;

use crate::bus::Bus;

/// Every field is a security-relevant choice; there is deliberately no
/// `Default` (pass-2 §1.2).
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Workspace the tools and sandbox operate on.
    pub workspace: PathBuf,
    /// redb database path.
    pub store_path: PathBuf,
    /// Model spec (`scripted:<path>`).
    pub model: String,
    /// Sandbox profile for the daemon's own bash mediation.
    pub sandbox: SandboxProfileName,
    /// Directory graph files are loaded from.
    pub graph_dir: PathBuf,
}

/// An executing or completed run.
pub struct RunHandle {
    /// Cancel token the node contexts observe.
    pub cancel: pii_core::CancelToken,
}

/// The server-side engine.
pub struct Engine {
    /// Run configuration.
    pub cfg: EngineConfig,
    /// Shared store.
    pub store: Arc<pii_store::Store>,
    /// Shared approval table (F5.9/F5.10): the `gate` node's decision
    /// source, and `approve.*`'s backing store.
    pub approvals: Arc<pii_store::approval::ApprovalStore>,
    /// Shared memory store (F6.7): the `memory` node's backing store, and
    /// `memory.*`'s backing store.
    pub memory: Arc<pii_store::MemStore>,
    /// Node registry (built-ins + `agent`).
    pub reg: NodeRegistry,
    /// The model provider.
    pub provider: Arc<dyn ModelProvider>,
    /// Tools available to sessions.
    pub tools: ToolSet,
    /// Event bus.
    pub bus: Arc<Bus>,
    /// Server start time.
    pub started_at: Instant,
    sessions: parking_lot::Mutex<BTreeMap<String, Session>>,
    runs: parking_lot::Mutex<BTreeMap<String, RunHandle>>,
    /// Session → runs it has spawned (for session-scoped `event.subscribe`).
    session_runs: parking_lot::Mutex<BTreeMap<String, Vec<String>>>,
    /// Set once by `pii daemon` (F5.1): `None` for a bare `Engine` (oneshot,
    /// tui, tests), `Some` once the daemon has built its runtime context.
    daemon_ctx: parking_lot::Mutex<Option<Arc<daemon::status::DaemonContext>>>,
}

/// A conversation session.
#[derive(Debug, Clone)]
pub struct Session {
    /// Session id.
    pub id: String,
    /// Working directory.
    pub cwd: String,
    /// Tool allowlist.
    pub tools: Vec<String>,
    /// Graph name override.
    pub graph: Option<String>,
    /// Status.
    pub status: SessionStatus,
    /// Transcript.
    pub messages: Vec<WireMessage>,
    /// Active run.
    pub run_id: Option<String>,
}

impl Engine {
    /// Builds an engine (opening the store eagerly).
    ///
    /// # Errors
    /// Store open failure or unknown model spec.
    pub fn new(cfg: EngineConfig) -> Result<Arc<Self>, PiiError> {
        let store = Arc::new(pii_store::Store::open(&cfg.store_path)?);
        let sibling = |name: &str| -> PathBuf {
            cfg.store_path
                .parent()
                .map_or_else(|| PathBuf::from(name), |p| p.join(name))
        };
        let approvals = Arc::new(pii_store::approval::ApprovalStore::open(&sibling(
            "approval.redb",
        ))?);
        let memory = Arc::new(pii_store::MemStore::open(&sibling("memory.redb"))?);
        // B1 (opus review): `PII_MODEL=ollama:<model>` now actually
        // constructs a live provider instead of failing `provider_from_spec`
        // (whose only working arm was `scripted:`). The transport is a
        // dedicated `pii-net::NetService`, scoped by a single grant for
        // exactly the configured Ollama host (loopback by default) — this
        // is the ONE place besides the daemon's own `[net]` config that
        // opens a `NetService`, and it exists specifically so the model
        // provider does not bypass the egress chokepoint `cargo xtask
        // net-check` enforces. `scripted:` specs (the common case in
        // tests) never touch this — `resolve_with_transport` only builds
        // the transport lazily, inside the `ollama:` branch.
        let provider = if cfg.model.starts_with("ollama:") {
            let ollama_host = pii_core::env_var("OLLAMA_HOST")
                .unwrap_or_else(|| String::from(pii_model::ollama::DEFAULT_OLLAMA_HOST));
            let host_only = ollama_host
                .trim_start_matches("http://")
                .trim_start_matches("https://")
                .split(':')
                .next()
                .unwrap_or("127.0.0.1")
                .to_owned();
            let mut grants = pii_net::grant::StaticGrants::default();
            grants.insert(
                "*",
                &format!("http:{host_only}"),
                pii_net::grant::NetGrant::open_local(),
            );
            let net = pii_net::NetService::new(pii_net::NetConfig {
                resolver: Arc::new(pii_net::SystemResolver),
                grants: Arc::new(grants),
                clock: Arc::new(pii_net::SystemClock),
            });
            pii_model::resolve_with_transport(&cfg.model, Arc::new(net))?
        } else {
            pii_model::provider_from_spec(&cfg.model)?
        };
        let bus = Bus::new();
        let reg = registry(
            &cfg,
            provider.clone(),
            bus.subscribe_tx(),
            Arc::clone(&approvals),
            Arc::clone(&memory),
        );
        let tools = ToolSet::builtin(cfg.workspace.clone());
        Ok(Arc::new(Self {
            cfg,
            store,
            approvals,
            memory,
            reg,
            provider,
            tools,
            bus,
            started_at: Instant::now(),
            sessions: parking_lot::Mutex::new(BTreeMap::new()),
            runs: parking_lot::Mutex::new(BTreeMap::new()),
            session_runs: parking_lot::Mutex::new(BTreeMap::new()),
            daemon_ctx: parking_lot::Mutex::new(None),
        }))
    }

    /// Attaches the daemon's runtime context (F5.1), set once at daemon
    /// startup after the engine is already `Arc`-shared.
    pub fn attach_daemon_ctx(&self, ctx: Arc<daemon::status::DaemonContext>) {
        *self.daemon_ctx.lock() = Some(ctx);
    }

    /// The daemon's runtime context, when this engine is running under
    /// `pii daemon` (F5.1). `None` for a bare `Engine`.
    #[must_use]
    pub fn daemon_ctx(&self) -> Option<Arc<daemon::status::DaemonContext>> {
        self.daemon_ctx.lock().clone()
    }

    /// Trips every active run's cancel token (F5.3 graceful shutdown).
    pub fn cancel_all(&self) {
        let ids: Vec<String> = self.runs.lock().keys().cloned().collect();
        for id in ids {
            self.cancel(&id);
        }
    }

    /// Creates a session (F2.7 `session.create`).
    pub fn create_session(&self, cwd: String, tools: Vec<String>, graph: Option<String>) -> String {
        let id = format!("s{}", self.sessions.lock().len() + 1);
        let allow = if tools.is_empty() {
            vec![
                String::from("fs_read"),
                String::from("fs_write"),
                String::from("edit"),
                String::from("multi_edit"),
                String::from("bash"),
            ]
        } else {
            tools
        };
        self.sessions.lock().insert(
            id.clone(),
            Session {
                id: id.clone(),
                cwd,
                tools: allow,
                graph,
                status: SessionStatus::Idle,
                messages: Vec::new(),
                run_id: None,
            },
        );
        id
    }

    /// Submits a prompt: allocates the run and returns BEFORE execution
    /// (< 50 ms, N4) — the graph runs on `spawn_blocking` (F2.3).
    ///
    /// # Errors
    /// Unknown session, or run start failure.
    pub fn prompt(
        self: &Arc<Self>,
        session_id: &str,
        content: &[ContentBlock],
        extra_input: Option<serde_json::Value>,
    ) -> Result<RunId, PiiError> {
        let prompt_text = content
            .iter()
            .map(|b| match b {
                ContentBlock::Text { text } => text.clone(),
            })
            .collect::<Vec<_>>()
            .join("\n");
        let session = {
            let mut map = self.sessions.lock();
            let s = map
                .get_mut(session_id)
                .ok_or_else(|| PiiError::NotFound(format!("session '{session_id}'")))?;
            s.status = SessionStatus::Running;
            s.messages.push(WireMessage {
                role: "user".to_owned(),
                content: prompt_text.clone(),
            });
            let session = s.clone();
            drop(map);
            session
        };
        let graph_name = session.graph.unwrap_or_else(|| "default".into());
        let spec = self.load_spec(&graph_name)?;
        let run_id = self.store.next_run_id()?;
        if let Some(s) = self.sessions.lock().get_mut(session_id) {
            s.run_id = Some(run_id.to_string());
        }
        self.session_runs
            .lock()
            .entry(session_id.to_owned())
            .or_default()
            .push(run_id.to_string());
        runs::spawn_run(
            Arc::clone(self),
            spec,
            run_id.clone(),
            prompt_text,
            session_id.to_owned(),
            extra_input,
        )?;
        Ok(run_id)
    }

    /// Starts a graph run directly (F6, opus review: `run.start`). Allocates
    /// the run and returns BEFORE execution (< 50 ms, N4). The input is the
    /// graph's run input; the prompt is derived from it for the agent node.
    ///
    /// # Errors
    /// Unknown graph, or run start failure.
    pub fn start_run(
        self: &Arc<Self>,
        graph: &str,
        input: &serde_json::Value,
    ) -> Result<RunId, PiiError> {
        let spec = self.load_spec(graph)?;
        let run_id = self.store.next_run_id()?;
        let prompt = input
            .get("prompt")
            .and_then(serde_json::Value::as_str)
            .map_or_else(|| input.to_string(), str::to_owned);
        runs::spawn_run(
            Arc::clone(self),
            spec,
            run_id.clone(),
            prompt,
            String::new(),
            None,
        )?;
        Ok(run_id)
    }

    /// Loads a graph by name: `graph_dir/<name>.toml`, falling back to the
    /// built-in `default` agent graph.
    ///
    /// Uses `self.reg` (B2, opus review) — the SAME registry that will
    /// actually execute the graph — so each node's default caps match what
    /// `pii-graph`'s dispatcher will enforce at runtime. Loading through
    /// plain `pii_graph::load_toml` here would silently default
    /// `agent`/`tool`/`gate`/`memory`/`crash` nodes to `CapKind::empty()`,
    /// since those kinds only exist on this engine's registry, not on
    /// `pii-graph`'s built-in one.
    ///
    /// # Errors
    /// [`PiiError::NotFound`] when neither exists; loader errors otherwise.
    pub fn load_spec(&self, name: &str) -> Result<GraphSpec, PiiError> {
        let path = self.cfg.graph_dir.join(format!("{name}.toml"));
        if path.exists() {
            let src = std::fs::read_to_string(&path)
                .map_err(|e| PiiError::Internal(format!("read {}: {e}", path.display())))?;
            return pii_graph::load_toml_with_registry(&src, &self.reg);
        }
        if name == "default" {
            return pii_graph::load_toml_with_registry(DEFAULT_TOML, &self.reg);
        }
        Err(PiiError::NotFound(format!("graph '{name}'")))
    }

    /// Registers a spawned run's cancel token.
    pub fn register_run(&self, run_id: &RunId, cancel: pii_core::CancelToken) {
        self.runs
            .lock()
            .insert(run_id.to_string(), RunHandle { cancel });
    }

    /// Releases the run handle once a run has reached a terminal disposition
    /// (F5.14/N4: every completed run must drop its `RunHandle`, or the
    /// `runs` map grows unboundedly under soak — the RSS gate's leak).
    pub fn unregister_run(&self, run_id: &str) {
        self.runs.lock().remove(run_id);
    }

    /// Trips a run's cancel token (idempotent, F2.7).
    pub fn cancel(&self, run_id: &str) {
        if let Some(h) = self.runs.lock().get(run_id) {
            h.cancel.cancel();
        }
        // F19: use the run's current journal seq so the cancellation event
        // does not poison the subscriber dedup cursor.
        let seq = self
            .store
            .get_run(&RunId::from(run_id))
            .ok()
            .flatten()
            .map_or(0, |h| h.seq);
        self.bus.broadcast_cancelled(run_id, seq);
        let mut map = self.sessions.lock();
        if let Some(s) = map
            .values_mut()
            .find(|s| s.run_id.as_deref() == Some(run_id))
        {
            s.status = SessionStatus::Idle;
            s.run_id = None;
        }
        drop(map);
        self.runs.lock().remove(run_id);
    }

    /// Session snapshot (F1.5 `session.state`).
    #[must_use]
    pub fn session_state(
        &self,
        session_id: &str,
    ) -> Option<pii_proto::session::SessionStateResult> {
        let map = self.sessions.lock();
        let s = map.get(session_id)?;
        let result = pii_proto::session::SessionStateResult {
            session_id: s.id.clone(),
            cwd: s.cwd.clone(),
            status: s.status,
            messages: s.messages.clone(),
            tokens_in: 0,
            tokens_out: 0,
            run_id: s.run_id.clone(),
        };
        drop(map);
        Some(result)
    }

    /// Session status transition after a run terminal.
    pub fn session_finished(&self, run_id: &str) {
        let mut map = self.sessions.lock();
        if let Some(s) = map
            .values_mut()
            .find(|s| s.run_id.as_deref() == Some(run_id))
        {
            s.status = SessionStatus::Idle;
            s.run_id = None;
        }
    }

    /// Records the assistant reply from a run terminal into the session
    /// transcript (F20, opus review): `session.state` must be a full
    /// snapshot, not just the user's prompts.
    pub fn record_assistant(&self, session_id: &str, terminal: &pii_abi::Terminal) {
        let text = match terminal {
            pii_abi::Terminal::Success { output } => output.to_string(),
            pii_abi::Terminal::Failure { error } => error.clone(),
            pii_abi::Terminal::Suspended { .. } => String::from("(suspended)"),
        };
        let mut map = self.sessions.lock();
        if let Some(s) = map.get_mut(session_id) {
            s.messages.push(WireMessage {
                role: "assistant".to_owned(),
                content: text,
            });
        }
    }

    /// The run ids spawned by a session, oldest first (for session-scoped
    /// `event.subscribe`). Empty when the session is unknown or has no runs.
    #[must_use]
    pub fn session_run_ids(&self, session_id: &str) -> Vec<String> {
        self.session_runs
            .lock()
            .get(session_id)
            .cloned()
            .unwrap_or_default()
    }
}

/// Builds the registry with the built-ins plus `agent`, `tool`, `gate` and
/// `memory` (F2.2, F5.9, F6.7).
#[must_use]
pub fn registry(
    cfg: &EngineConfig,
    provider: Arc<dyn ModelProvider>,
    live: tokio::sync::broadcast::Sender<pii_proto::run::SeqEvent>,
    approvals: Arc<pii_store::approval::ApprovalStore>,
    memory: Arc<pii_store::MemStore>,
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
    reg.register(Arc::new(nodes::memory::MemoryFactory { store: memory }));
    reg.register(Arc::new(nodes::crash::DebugCrashFactory));
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
    reg
}

/// The built-in session graph: one `agent` node wired `ok → done`.
const DEFAULT_TOML: &str = r#"
name = "default"
entry = "agent"
caps = ["fs_read", "fs_write", "proc"]

[[node]]
id = "agent"
kind = "agent"
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
