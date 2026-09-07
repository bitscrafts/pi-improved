//! `pii-server`: the daemon side of the boundary (F2).
//!
//! The engine owns the store, the node registry (built-ins plus `agent`),
//! the model provider, and the tool set. Runs execute on blocking threads;
//! the journal→bus bridge is a [`pii_core::Observer`] so **the Phase-1
//! journaler stays the only redb writer** (the spec instructs reuse of the
//! existing run entry rather than a second journaling path).

pub mod agent;
pub mod bus;
pub mod compaction;
pub mod conn;
pub mod daemon;
pub mod dispatch;
pub mod loopback;
pub mod nodes;
pub mod registry;
pub mod runs;
pub mod session_durability;
pub mod subscribe;
pub mod tool;
pub mod uds;

pub use registry::{lint_registry, registry};

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
    /// Shared heal ledger (S0.1): the `heal_detect` node's backing store,
    /// and `heal.*`'s backing store.
    pub heals: Arc<pii_store::heal::HealStore>,
    /// Shared memory store (F6.7): the `memory` node's backing store, and
    /// `memory.*`'s backing store.
    pub memory: Arc<pii_store::MemStore>,
    /// Shared supervisor pool (F3.7): filled by `Daemon::build` from
    /// `[[worker]]` config; the `worker` node's factory holds the same Arc.
    pub workers: Arc<parking_lot::Mutex<BTreeMap<String, pii_worker::Supervisor>>>,
    /// Node registry (built-ins + `agent`).
    pub reg: NodeRegistry,
    /// The model provider.
    pub provider: Arc<dyn ModelProvider>,
    /// Tools available to sessions.
    pub tools: ToolSet,
    /// Event bus.
    pub bus: Arc<Bus>,
    /// The Ollama host a live `ollama:` provider's `NetService` was scoped
    /// to (F1/B1 follow-up, 2026-09-05) — `None` for `scripted:`. `runs.rs`
    /// uses this to grant a run's `CapSet` exactly the `NET` capability
    /// (and only that host) an `agent` node needs to actually reach the
    /// model: `Engine::new` wired the transport's own egress grant
    /// correctly (B1), but no RUN ever received `CapKind::NET` on its own
    /// caps, so `dial()`'s `ctx.net()` gate denied every live-model call
    /// before it reached the (correctly configured) transport. This field
    /// closes that gap without widening any run's `NET` scope beyond the
    /// single host the daemon was actually configured to call.
    pub ollama_net_host: Option<String>,
    /// Server start time.
    pub started_at: Instant,
    pub(crate) sessions: parking_lot::Mutex<BTreeMap<String, Session>>,
    runs: parking_lot::Mutex<BTreeMap<String, RunHandle>>,
    /// Session → runs it has spawned (for session-scoped `event.subscribe`).
    session_runs: parking_lot::Mutex<BTreeMap<String, Vec<String>>>,
    /// Set once by `pii daemon` (F5.1): `None` for a bare `Engine` (oneshot,
    /// tui, tests), `Some` once the daemon has built its runtime context.
    daemon_ctx: parking_lot::Mutex<Option<Arc<daemon::status::DaemonContext>>>,
}

/// A conversation session.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
        let heals = Arc::new(pii_store::heal::HealStore::open(&sibling("heal.redb"))?);
        let memory = Arc::new(pii_store::MemStore::open(&sibling("memory.redb"))?);
        let workers = Arc::new(parking_lot::Mutex::new(BTreeMap::<
            String,
            pii_worker::Supervisor,
        >::new()));
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
        let mut ollama_net_host: Option<String> = None;
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
            ollama_net_host = Some(host_only.clone());
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
            Arc::clone(&heals),
            Arc::clone(&memory),
            Arc::clone(&workers),
        );
        let tools = ToolSet::builtin(cfg.workspace.clone());
        Ok(Arc::new(Self {
            cfg,
            store,
            approvals,
            heals,
            memory,
            workers,
            reg,
            provider,
            tools,
            bus,
            ollama_net_host,
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
        // R1 (spec 16 part2): mint from the durable counter so ids never
        // collide after a removal/restart; persist the session so a daemon
        // restart re-seeds it (R2 of the Phase-4 plan — "daemon restart is
        // amnesia").
        let n = self.store.next_session_id().unwrap_or(1);
        let id = format!("s{n}");
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
        self.persist_session(&id);
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
        let (session, history) = {
            let mut map = self.sessions.lock();
            let s = map
                .get_mut(session_id)
                .ok_or_else(|| PiiError::NotFound(format!("session '{session_id}'")))?;
            s.status = SessionStatus::Running;
            // 2026-09-05 (session memory): capture the transcript BEFORE
            // pushing this turn's user message, so `history` below is
            // exactly the prior turns — the current prompt is threaded
            // separately (as `prompt`, unchanged) and the agent node
            // appends it itself, so it must not also appear in history
            // or it would be duplicated in the model's message list.
            let history = s.messages.clone();
            s.messages.push(WireMessage {
                role: "user".to_owned(),
                content: prompt_text.clone(),
            });
            let session = s.clone();
            drop(map);
            (session, history)
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
        // P4.2 (spec 16): persist the run_id/status transition so the session
        // reflects the running run after a restart too.
        self.persist_session(session_id);
        // 2026-09-05 (session memory): merge the prior transcript into
        // extra_input under `history` (see `merge_history` doc comment).
        let extra_input = Some(merge_history(extra_input, &history));
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
    /// graph's run input — it is forwarded to the graph (`$.input`) so
    /// parameterised graphs (verify-fix's `test_command`/`workspace`,
    /// heal-fix's `scope_root`/`worktree`/`test_command`) receive their
    /// params — and the prompt is derived from it for the agent node.
    ///
    /// # Errors
    /// Unknown graph, invalid `scope_root`, or run start failure.
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
            Some(input.clone()),
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
            return pii_graph::load_toml_with_registry(registry::DEFAULT_TOML, &self.reg);
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
}

fn merge_history(
    extra_input: Option<serde_json::Value>,
    history: &[WireMessage],
) -> serde_json::Value {
    let history_json = serde_json::json!(
        history
            .iter()
            .map(|m| serde_json::json!({"role": m.role, "content": m.content}))
            .collect::<Vec<_>>()
    );
    match extra_input {
        Some(serde_json::Value::Object(mut obj)) => {
            obj.insert(String::from("history"), history_json);
            serde_json::Value::Object(obj)
        }
        Some(other) => other,
        None => serde_json::json!({ "history": history_json }),
    }
}
