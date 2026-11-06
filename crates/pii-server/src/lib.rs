//! `pii-server`: the daemon side of the boundary (F2).
//!
//! The engine owns the store, the node registry (built-ins plus `agent`),
//! the model provider, and the tool set. Runs execute on blocking threads;
//! the journal→bus bridge is a [`pii_core::Observer`] so **the Phase-1
//! journaler stays the only redb writer** (the spec instructs reuse of the
//! existing run entry rather than a second journaling path).

// Out of scope for spec 77 (lint enabled for pii-graph only); the exit-criteria
// clippy run passes -W for pii-server too, so existing best-effort discards
// here are allowed until a later per-crate sweep.

pub mod daemon;
pub mod engine;
mod engine_attach;
pub mod node_kinds;
pub mod tiers;

pub mod observed_provider;
use observed_provider::observed_provider;

pub use engine::registry::{lint_registry, lint_registry_with_log, registry};

/// Default cap on `session_runs` entries retained per session (D3/D6, 026
/// report 5.0.4) when a caller does not set `EngineConfig.session_runs_cap`
/// from config explicitly.
pub const DEFAULT_SESSION_RUNS_CAP: usize = 256;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use pii_abi::{PiiError, RunId};
pub use pii_graph::node::NodeRegistry;
use pii_graph::spec::GraphSpec;
use pii_model::ModelProvider;
use pii_proto::session::{ContentBlock, SessionStatus, WireMessage};
use pii_tools::ToolSet;
use pii_tools::sandbox::SandboxProfileName;

use crate::engine::bus::Bus;

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
    /// `[evolve.routing]` step -> model policy (spec 28). Empty = no routing.
    pub route_policy: BTreeMap<String, String>,
    /// Spec 215: the most capability any graph or node may request; `None` = no ceiling.
    pub grant_ceiling: Option<pii_core::CapKind>,
    /// `[[model_tier]]` table (spec 114). Empty = no tiers.
    pub tiers: Vec<daemon::config::ModelTierSection>,
    /// Broker URL for the `mqtt` TOOL (the model's door to the devices). `None` = the tool is absent, which is the
    /// honest default: a model with no broker configured must not believe it can drive the house.
    pub mqtt_tool_url: Option<String>,
    /// Max runs retained per session in the in-memory `session_runs` index
    /// (D3/D6, 026 report 5.0.4 config key) — oldest run ids are evicted
    /// (LRU by insertion order) once a session's run list exceeds this so
    /// a long-lived session cannot grow the index unboundedly. Config key:
    /// `[core].session_runs_cap` (`daemon/config.rs`); callers outside the
    /// daemon (tests, `pii -p`, `pii tui`) pass
    /// [`DEFAULT_SESSION_RUNS_CAP`] explicitly since `EngineConfig`
    /// deliberately has no `Default`.
    pub session_runs_cap: usize,
    /// `[llama] host` (spec 129): base URL of the llama.cpp server a
    /// `llamacpp-server:` model dials. `None` = loopback default.
    pub llama_host: Option<String>,
}

pub use graph_paths::{find_graph, graph_not_found, graph_search_dirs, resolve_graph_dir};

pub mod graph_paths;

/// An executing or completed run.
pub struct RunHandle {
    /// Cancel token the node contexts observe.
    pub cancel: pii_core::CancelToken,
    /// Whether a run thread has claimed the run yet (spec 308 R3).
    pub claim: engine::run_pool::Claim,
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
    /// Shared queue store (`queue.redb`, beside the main store). Held here so
    /// health can report it; the queue nodes hold their own clone.
    pub queue: Arc<pii_store::queue::QueueStore>,
    /// Shared memory store (F6.7): the `memory` node's backing store, and
    /// `memory.*`'s backing store.
    pub memory: Arc<pii_store::MemStore>,
    /// Shared supervisor pool (F3.7): filled by `Daemon::build` from
    /// `[[worker]]` config; the `worker` node's factory holds the same Arc.
    pub workers: Arc<parking_lot::Mutex<BTreeMap<String, pii_worker::Supervisor>>>,
    /// The MQTT connections (spec 117): filled by `Daemon::build` from
    /// `[[mqtt]]`; the `mqtt` node's factory holds the same Arc.
    pub mqtt: Arc<pii_channels::mqtt::MqttHub>,
    /// The configured channels (spec 128): `Daemon::build` fills it; the `post` node shares it.
    pub channel_hub: Arc<pii_channels::post::ChannelHub>,
    /// The hardware HAL slot (spec 121): empty unless `[hal]` is configured;
    /// `Daemon::build` fills it and the `hal` node's factory holds the same Arc.
    pub hal: Arc<pii_hal::HalHub>,
    /// The WASM plugin host slot (spec 122): empty unless `[wasm]` is
    /// configured; `Daemon::build` fills it and the `wasm` node's factory holds
    /// the same Arc.
    pub wasm: Arc<pii_wasm::WasmHub>,
    /// The entity-store slot (spec 126): `Daemon::build` fills it; the `remember` and
    /// `recall` nodes' factories hold the same Arc.
    pub entities: Arc<pii_memory::entity::EntityHub>,
    /// Node registry (built-ins + `agent`).
    pub reg: NodeRegistry,
    /// The model provider.
    pub provider: Arc<dyn ModelProvider>,
    /// The model tiers (spec 114), each with its own provider.
    pub tiers: Arc<tiers::TierTable>,
    /// Tools available to sessions.
    pub tools: ToolSet,
    /// Event bus.
    pub bus: Arc<Bus>,
    /// The host a live local (`llamacpp-server:`) or hosted provider's
    /// `NetService` was scoped to (F1/B1 follow-up, 2026-09-05; renamed from
    /// the retired ollama path, spec 129) — `None` for `scripted:`. `runs.rs`
    /// uses this to grant a run's `CapSet` exactly the `NET` capability
    /// (and only that host) an `agent` node needs to actually reach the
    /// model: `Engine::new` wired the transport's own egress grant
    /// correctly (B1), but no RUN ever received `CapKind::NET` on its own
    /// caps, so `dial()`'s `ctx.net()` gate denied every live-model call
    /// before it reached the (correctly configured) transport. This field
    /// closes that gap without widening any run's `NET` scope beyond the
    /// single host the daemon was actually configured to call.
    pub local_net_host: Option<String>,
    /// Server start time.
    pub started_at: Instant,
    pub(crate) sessions: parking_lot::Mutex<BTreeMap<String, Session>>,
    runs: parking_lot::Mutex<BTreeMap<String, RunHandle>>,
    /// The threads runs execute on (spec 308): runs never take the shared blocking pool.
    pub run_pool: Arc<engine::run_pool::RunPool>,
    /// Shared handle the `spawn` node uses to launch child runs (spec 32 R1).
    /// `None` until the daemon (or a test) calls [`Engine::attach_spawn_engine`]
    /// after construction — the registry is built inside `Engine::new`, so the
    /// factory cannot hold the `Arc<Engine>` directly (chicken-and-egg). Same
    /// pattern as the `worker` node's shared pool.
    spawn_handle: node_kinds::nodes::spawn::SpawnEngineHandle,
    /// Session → runs it has spawned (for session-scoped `event.subscribe`).
    session_runs: parking_lot::Mutex<BTreeMap<String, Vec<String>>>,
    /// Set once by `pii daemon` (F5.1): `None` for a bare `Engine` (oneshot,
    /// tui, tests), `Some` once the daemon has built its runtime context.
    daemon_ctx: parking_lot::Mutex<Option<Arc<daemon::status::DaemonContext>>>,
    /// Observer for events raised outside a run's own `Ctx` (D3/D6, 026
    /// report 5.0.4) — currently just session-persist failures. `None`
    /// (the default) means [`NullObserver`][pii_core::NullObserver];
    /// [`Engine::attach_observer`] lets a daemon or test wire a real one
    /// without threading it through every constructor (same pattern as
    /// [`Engine::attach_daemon_ctx`]).
    obs: parking_lot::Mutex<Option<Arc<dyn pii_core::Observer>>>,
    /// Trajectory sink (spec 103); `None` (default) records nothing.
    trajectory: parking_lot::Mutex<Option<Arc<pii_observe::TrajectorySink>>>,
    /// The trajectory root the promotion lifecycle measures outcomes against (spec 125).
    learn_traj_root: parking_lot::Mutex<Option<PathBuf>>,
    audit: parking_lot::Mutex<Option<Arc<dyn pii_core::Observer>>>,
    /// Spec 116 (measurement only): when set, runs carry no tool-call observer.
    observers_detached: std::sync::atomic::AtomicBool,
    /// The contract-observer switch (spec 108): handed to the store, the
    /// model provider and the network service at construction; unattached
    /// (drops everything) until [`Engine::attach_log`].
    log: Arc<pii_observe::SwitchObserver>,
    /// Last-seen state per subsystem, so `sys.status` polls report transitions
    /// as `health.state.changed` (spec 109 R2).
    health_watch: pii_observe::HealthWatch,
    /// Run-lifecycle latency by outcome (spec 115 R4), recorded in-process.
    pub(crate) latency: pii_observe::RunLatency,
    /// RSS-per-run trend over recent `sys.status` polls (spec 116 R2).
    pub(crate) rss_trend: pii_observe::RssTrend,
    /// Quarantine refusals vs refusals recorded in the decision ledger (spec 131).
    pub refusals: engine::refusal::RefusalCounts,
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
    #[allow(clippy::too_many_lines)] // reason: one linear wiring of the engine's shared resources
    pub fn new(cfg: EngineConfig) -> Result<Arc<Self>, PiiError> {
        let store = Arc::new(pii_store::Store::open(&cfg.store_path)?);
        store.set_origin(daemon::since_deploy::process_origin());
        let log = Arc::new(pii_observe::SwitchObserver::new());
        store.set_observer(log.clone());
        // Spec 370: every run write moves the health record in its own transaction.
        store.set_append_observer(daemon::sys_status::observe_append);
        let sibling = |name: &str| -> PathBuf {
            cfg.store_path
                .parent()
                .map_or_else(|| PathBuf::from(name), |p| p.join(name))
        };
        let approvals = Arc::new(pii_store::approval::ApprovalStore::open(&sibling(
            "approval.redb",
        ))?);
        let heals = Arc::new(pii_store::heal::HealStore::open(&sibling("heal.redb"))?);
        let queue = Arc::new(pii_store::queue::QueueStore::open(&sibling("queue.redb"))?);
        let memory = Arc::new(pii_store::MemStore::open(&sibling("memory.redb"))?);
        let workers = Arc::new(parking_lot::Mutex::new(BTreeMap::<
            String,
            pii_worker::Supervisor,
        >::new()));
        let (provider, local_net_host) =
            observed_provider(&cfg.model, &log, cfg.llama_host.as_deref())?;
        let tiers = Arc::new(tiers::TierTable::build(
            &cfg.tiers,
            |spec| observed_provider(spec, &log, cfg.llama_host.as_deref()).map(|(p, _)| p),
            |tier, model, why| {
                pii_observe::Observer::event(
                    &*log,
                    &pii_observe::Event::new(
                        "model",
                        pii_observe::Level::Error,
                        pii_observe::names::MODEL_TIER_UNAVAILABLE,
                    )
                    .field("tier", tier)
                    .field("model", model)
                    .field("error", why),
                );
            },
        ));
        let bus = Bus::new();
        let spawn_engine: node_kinds::nodes::spawn::SpawnEngineHandle =
            Arc::new(parking_lot::Mutex::new(None));
        let mqtt = Arc::new(pii_channels::mqtt::MqttHub::default());
        let mut reg = registry(
            &cfg,
            provider.clone(),
            bus.subscribe_tx(),
            Arc::clone(&approvals),
            Arc::clone(&heals),
            Arc::clone(&memory),
            Arc::clone(&workers),
            cfg.route_policy.clone(),
            Arc::clone(&spawn_engine),
            log.clone(),
            Arc::clone(&tiers),
            node_kinds::nodes::coder_container::PODMAN_BIN.into(),
        );
        engine::registry::register_queue_nodes(
            &mut reg,
            Arc::clone(&queue),
            Some(Arc::clone(&memory)),
            Some(Arc::clone(&heals)),
            Some(Arc::new(node_kinds::nodes::heal::VerdictRunner {
                workspace: cfg.workspace.clone(),
            })),
            Some(log.clone()),
        );
        reg.register(Arc::new(node_kinds::nodes::mqtt::MqttFactory {
            hub: Arc::clone(&mqtt),
        }));
        let channel_hub = engine::registry::register_post_node(&mut reg, log.clone());
        let hal = Arc::new(pii_hal::HalHub::default());
        reg.register(Arc::new(node_kinds::nodes::hal::HalFactory {
            hub: Arc::clone(&hal),
            log: log.clone(),
        }));
        let wasm = Arc::new(pii_wasm::WasmHub::default());
        reg.register(Arc::new(node_kinds::nodes::wasm::WasmFactory {
            hub: Arc::clone(&wasm),
            log: log.clone(),
        }));
        let entities = engine::registry::register_entity_nodes(&mut reg);
        let mut tools = ToolSet::builtin(cfg.workspace.clone());
        if let Some(url) = cfg.mqtt_tool_url.as_deref() {
            let observer: Arc<dyn pii_observe::Observer> = log.clone();
            tools = tools.with_mqtt(url, observer)?;
        }
        let run_pool = Arc::new(engine::run_pool::RunPool::new(
            engine::run_pool::configured_threads(),
        )?);
        Ok(Arc::new(Self {
            cfg,
            store,
            run_pool,
            approvals,
            heals,
            queue,
            memory,
            workers,
            mqtt,
            channel_hub,
            hal,
            wasm,
            entities,
            reg,
            provider,
            tiers,
            tools,
            bus,
            local_net_host,
            started_at: Instant::now(),
            sessions: parking_lot::Mutex::new(BTreeMap::new()),
            runs: parking_lot::Mutex::new(BTreeMap::new()),
            spawn_handle: Arc::clone(&spawn_engine),
            session_runs: parking_lot::Mutex::new(BTreeMap::new()),
            observers_detached: std::sync::atomic::AtomicBool::new(false),
            daemon_ctx: parking_lot::Mutex::new(None),
            obs: parking_lot::Mutex::new(None),
            trajectory: parking_lot::Mutex::new(None),
            learn_traj_root: parking_lot::Mutex::new(None),
            audit: parking_lot::Mutex::new(None),
            log,
            health_watch: pii_observe::HealthWatch::new(),
            latency: pii_observe::RunLatency::default(),
            refusals: engine::refusal::RefusalCounts::default(),
            rss_trend: pii_observe::RssTrend::new(),
        }))
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
        self.trim_session_runs(session_id);
        // P4.2 (spec 16): persist the run_id/status transition so the session
        // reflects the running run after a restart too.
        self.persist_session(session_id);
        // 2026-09-05 (session memory): merge the prior transcript into
        // extra_input under `history` (see `merge_history` doc comment).
        let extra_input = Some(merge_history(extra_input, &history));
        engine::runs::spawn_run(
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
        self.start_run_with(graph, input, |_| {})
    }

    /// [`Engine::start_run`] with `on_id` called once the id is allocated and before the run is spawned (spec 305 R4): a caller that
    /// registers the run from its id can do so before the run can finish.
    ///
    /// # Errors
    /// As [`Engine::start_run`].
    pub fn start_run_with(
        self: &Arc<Self>,
        graph: &str,
        input: &serde_json::Value,
        on_id: impl FnOnce(&RunId),
    ) -> Result<RunId, PiiError> {
        let (src, woven) = self.load_woven(graph)?;
        // Spec 288: a call's `needs` the input cannot supply refuses here, before any run exists.
        engine::preflight::check_needs(&woven.calls, &src, input)?;
        engine::preflight::check_gate_deadlines(&woven.spec)?;
        let spec = woven.spec;
        let run_id = self.store.next_run_id()?;
        on_id(&run_id);
        let prompt = input
            .get("prompt")
            .and_then(serde_json::Value::as_str)
            .map_or_else(|| input.to_string(), str::to_owned);
        engine::runs::spawn_run(
            Arc::clone(self),
            spec,
            run_id.clone(),
            prompt,
            String::new(),
            Some(input.clone()),
        )?;
        Ok(run_id)
    }

    /// Loads a graph by name: the first of [`graph_search_dirs`] that holds
    /// `<name>.toml`, falling back to the built-in `default` agent graph.
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
    /// [`PiiError::NotFound`] naming every dir searched; loader errors otherwise.
    pub fn load_spec(&self, name: &str) -> Result<GraphSpec, PiiError> {
        self.load_woven(name).map(|(_, w)| w.spec)
    }

    /// [`Engine::load_spec`] with the host's source text and the call records the load expanded (spec 288: the start
    /// preflight reads both). The `default` graph has no source text and no calls.
    ///
    /// # Errors
    /// As [`Engine::load_spec`].
    pub fn load_woven(&self, name: &str) -> Result<(String, pii_graph::fragment::Woven), PiiError> {
        let dirs = graph_search_dirs(&self.cfg.graph_dir, &self.cfg.store_path);
        if let Some(path) = find_graph(&dirs, name) {
            let src = std::fs::read_to_string(&path)
                .map_err(|e| PiiError::Internal(format!("read {}: {e}", path.display())))?;
            // Spec 219: a graph that attaches fragments is woven here, before it is validated or run.
            let dir = path.parent().unwrap_or(&self.cfg.graph_dir);
            let fragments = dir.join("fragments");
            let woven =
                pii_graph::fragment::load_host_with_graphs(&src, &fragments, &dirs, &self.reg)?;
            return Ok((src, woven));
        }
        if name == "default" {
            let woven = pii_graph::fragment::Woven {
                spec: pii_graph::load_toml_with_registry(
                    engine::registry::DEFAULT_TOML,
                    &self.reg,
                )?,
                sequence: Vec::new(),
                calls: Vec::new(),
            };
            return Ok((String::new(), woven));
        }
        Err(graph_not_found(name, &dirs))
    }

    /// Registers a spawned run's cancel token; the returned claim is the run's place in the run pool.
    pub fn register_run(
        &self,
        run_id: &RunId,
        cancel: pii_core::CancelToken,
    ) -> engine::run_pool::Claim {
        self.register_with(run_id, cancel, engine::run_pool::Claim::executing())
    }

    /// As [`Engine::register_run`] for a run that waits in the run pool (spec 308): its claim is
    /// `QUEUED` until a pool thread starts it, so a cancel can take it first.
    pub fn register_queued_run(
        &self,
        run_id: &RunId,
        cancel: pii_core::CancelToken,
    ) -> engine::run_pool::Claim {
        self.register_with(run_id, cancel, engine::run_pool::Claim::default())
    }

    fn register_with(
        &self,
        run_id: &RunId,
        cancel: pii_core::CancelToken,
        claim: engine::run_pool::Claim,
    ) -> engine::run_pool::Claim {
        self.runs.lock().insert(
            run_id.to_string(),
            RunHandle {
                cancel,
                claim: claim.clone(),
            },
        );
        claim
    }

    /// Spec 308 R3: claims a run that no pool thread has started, so it will never execute.
    /// False when the run is unknown, already running, or already cancelled.
    #[must_use]
    pub fn cancel_queued(&self, run_id: &str) -> bool {
        self.runs
            .lock()
            .get(run_id)
            .is_some_and(|h| h.claim.cancel_queued())
    }

    /// Releases the run handle once a run has reached a terminal disposition
    /// (F5.14/N4: every completed run must drop its `RunHandle`, or the
    /// `runs` map grows unboundedly under soak — the RSS gate's leak).
    pub fn unregister_run(&self, run_id: &str) {
        self.runs.lock().remove(run_id);
    }

    /// True while this process holds a live handle for `run_id`.
    #[must_use]
    pub fn is_in_flight(&self, run_id: &str) -> bool {
        self.runs.lock().contains_key(run_id)
    }

    /// Trips a run's cancel token and keeps its handle: the run leaves flight only when its node ends
    /// (spec 243). [`Engine::cancel`] drops the handle, so it cannot be waited on.
    pub fn trip_cancel(&self, run_id: &str) {
        if let Some(h) = self.runs.lock().get(run_id) {
            h.cancel.cancel();
        }
    }

    /// Trips a run's cancel token (idempotent, F2.7).
    pub fn cancel(&self, run_id: &str) {
        self.trip_cancel(run_id);
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

#[cfg(test)]
mod local_model_tests;
