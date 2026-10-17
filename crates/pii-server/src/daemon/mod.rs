//! The daemon (F5.1): lifecycle, timer wheel, cron, approvals, channel router.
//!
//! `pii daemon` builds one [`Daemon`] and calls [`Daemon::spawn_background`],
//! then races [`ShutdownSignal`] against its own accept loop.

mod adapters;
pub mod approvals;
mod channel_wiring;
pub mod config;
pub mod identity;
mod lag;
mod learn_loop;
#[cfg(test)]
mod learn_rpc_tests;
#[cfg(test)]
mod refusal_tests;
pub mod router;
#[cfg(test)]
mod sanction_tests;
pub mod scheduler;
pub mod status;
mod suspend;

use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;

use pii_abi::PiiError;
use pii_core::Clock as _;
use pii_proto::event::{Event, WireTerminal};

use crate::Engine;
use status::{ChannelRuntime, CronEntry, DaemonContext};

/// Attaches the daemon context to `engine`, and (spec 125) records the
/// trajectory root the promotion lifecycle measures outcomes against.
fn attach_ctx(engine: &Engine, ctx: &Arc<DaemonContext>, cfg: &config::Config) {
    engine.attach_daemon_ctx(Arc::clone(ctx));
    if let Some(root) = pii_core::env_var(pii_core::TRAJECTORY_DIR_ENV)
        .or_else(|| cfg.core.trajectory_dir.clone())
        .filter(|d| !d.is_empty())
    {
        engine.attach_learn_traj_root(PathBuf::from(root));
    }
}

/// Owns everything `pii daemon` starts on top of a bare [`Engine`] (F5.1).
pub struct Daemon {
    /// The shared engine.
    pub engine: Arc<Engine>,
    /// The daemon's runtime context (also attached to `engine`).
    pub ctx: Arc<DaemonContext>,
    /// The timer scheduler.
    pub scheduler: Arc<scheduler::Scheduler>,
    /// Period of the unattended learn pass (spec 139); `None` = off.
    learn_interval: Option<std::time::Duration>,
}

impl Daemon {
    /// Builds the daemon from a parsed config: the egress service, the
    /// supervised workers, the configured channels and cron entries, the
    /// timer wheel, and the channel-cursor table. Attaches the resulting
    /// context to `engine` so `sys.status`/`worker.list`/etc. see it.
    ///
    /// # Errors
    /// A malformed host pattern, an unknown channel kind, a missing
    /// Discord env var, or a store-open failure.
    pub fn build(
        engine: Arc<Engine>,
        cfg: &config::Config,
        runtime_dir: &Path,
        config_path: PathBuf,
        clock: Arc<dyn pii_core::Clock>,
    ) -> Result<Arc<Self>, PiiError> {
        Self::build_with_env(
            engine,
            cfg,
            runtime_dir,
            config_path,
            clock,
            &pii_core::env_var,
        )
    }

    /// [`Self::build`] over an arbitrary variable lookup (so tests need no process-global env).
    /// A `discord` channel whose credentials are absent is not built: it is reported at startup
    /// and the daemon runs without it (spec 157).
    ///
    /// # Errors
    /// As [`Self::build`].
    // reason: one linear wiring sequence; splitting would only hide the order.
    #[allow(clippy::too_many_lines)]
    pub fn build_with_env(
        engine: Arc<Engine>,
        cfg: &config::Config,
        runtime_dir: &Path,
        config_path: PathBuf,
        clock: Arc<dyn pii_core::Clock>,
        env: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Arc<Self>, PiiError> {
        let grants = Arc::new(config::ConfigGrants::from_section(&cfg.net)?);
        let net = Arc::new(pii_net::NetService::new(pii_net::NetConfig {
            resolver: Arc::new(pii_net::SystemResolver),
            grants: Arc::clone(&grants) as pii_net::grant::Grants,
            clock: Arc::clone(&clock),
        }));
        net.set_observer(engine.log());

        let workers = start_workers(&engine, cfg);

        let mut channels = BTreeMap::new();
        for c in &cfg.channel {
            if channel_wiring::skip_unconfigured(&engine, c, env) {
                continue;
            }
            let channel = build_channel(c, &net, &grants, runtime_dir, &engine, env)?;
            channels.insert(
                c.name.clone(),
                ChannelRuntime {
                    channel,
                    poll_interval_ms: c.poll_interval_ms,
                    graph: c.graph.clone(),
                },
            );
        }

        for m in &cfg.mqtt {
            if channels.contains_key(&m.name) {
                return Err(PiiError::Invalid(format!(
                    "mqtt '{}': name already used by another channel",
                    m.name
                )));
            }
            let conn = adapters::mqtt(m, &engine.log())?;
            engine.mqtt.insert(Arc::clone(&conn));
            channels.insert(
                m.name.clone(),
                ChannelRuntime {
                    channel: conn,
                    poll_interval_ms: m.poll_interval_ms,
                    graph: m.graph.clone(),
                },
            );
        }

        adapters::install_devices(&engine, cfg, runtime_dir)?;

        let mut cron = BTreeMap::new();
        for c in &cfg.cron {
            // Spec 176: a non-timer entry would re-run the instant it finishes.
            if let Ok(spec) = engine.load_spec(&c.graph) {
                let entry_kind = spec.node(&spec.entry).map(|n| n.kind.as_str());
                if entry_kind != Some("timer") {
                    return Err(PiiError::Invalid(format!(
                        "cron '{}': graph '{}' must start at a timer node (entry is {}); \
                         it would re-run as fast as it finishes",
                        c.name,
                        c.graph,
                        entry_kind.unwrap_or("missing"),
                    )));
                }
            }
            cron.insert(
                c.name.clone(),
                CronEntry {
                    name: c.name.clone(),
                    graph: c.graph.clone(),
                    schedule: c.schedule.clone(),
                    input: c.input.clone(),
                },
            );
        }

        let channel_store = Arc::new(pii_store::channel::ChannelStore::open(
            &runtime_dir.join("channel.redb"),
        )?);
        let wheel = Arc::new(pii_store::timer::TimerWheel::open(
            &runtime_dir.join("timer.redb"),
        )?);

        // Phase 5.0 D2: the channel fs scope is rooted under the resolved
        // runtime dir, never a literal path. A missing directory is a
        // config error at daemon start (fail loud), not a silent per-poll
        // `Denied` discovered later.
        let channels_scope = resolve_channels_scope(runtime_dir)?;

        let ctx = Arc::new(DaemonContext {
            net,
            workers,
            channels: Mutex::new(channels),
            channel_store,
            timer_store: Arc::clone(&wheel),
            cron: Mutex::new(cron),
            cron_runs: Mutex::new(BTreeMap::new()),
            channel_runs: Mutex::new(BTreeMap::new()),
            crashes_dir: runtime_dir.join("crashes"),
            config_path,
            channels_scope,
            runtime_dir: runtime_dir.to_path_buf(),
        });
        attach_ctx(&engine, &ctx, cfg);
        // Spec 32 R1: give the `spawn` node the engine handle it needs to
        // launch child runs (the registry was built inside `Engine::new`,
        // before the Arc existed).
        engine.attach_spawn_engine(Arc::clone(&engine));

        let scheduler = scheduler::Scheduler::new(Arc::clone(&engine), wheel, clock);
        Ok(Arc::new(Self {
            engine,
            ctx,
            scheduler,
            learn_interval: cfg.core.learn_interval(),
        }))
    }

    /// Installs the panic hook (F5.4): writes
    /// `<runtime_dir>/crashes/<now_ms>.json` synchronously before delegating
    /// to the previous hook. Must be called once, before anything is served.
    pub fn install_panic_hook(&self) {
        let crashes_dir = self.ctx.crashes_dir.clone();
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            write_crash_file(&crashes_dir, info);
            prev(info);
        }));
    }

    /// Starts every configured cron entry once (F5.8): the graph's own
    /// entry `timer` node suspends immediately; the scheduler and the
    /// reactive loop drive every later firing.
    pub fn start_cron_entries(&self) {
        let entries: Vec<CronEntry> = self.ctx.cron.lock().values().cloned().collect();
        for e in entries {
            start_cron(&self.engine, &self.ctx, &e);
        }
    }

    /// Spawns every background subsystem (F5.1): the scheduler loop, one
    /// poller per channel, and the reactive suspend/finish handler. Returns
    /// their join handles so the caller can abort them during shutdown.
    pub fn spawn_background(self: &Arc<Self>) -> Vec<tokio::task::JoinHandle<()>> {
        let mut handles = vec![tokio::spawn(Arc::clone(&self.scheduler).run())];
        handles.extend(router::spawn_all(&self.engine, &self.ctx));
        handles.push(self.spawn_reactive_loop());
        handles.push(self.spawn_promotion_loop());
        handles.extend(self.spawn_learn_loop());
        handles
    }

    /// Spec 125 R4/R5: the promotion lifecycle sweep (see [`learn_loop::spawn_promotion`]).
    fn spawn_promotion_loop(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        learn_loop::spawn_promotion(Arc::clone(&self.engine), self.ctx.runtime_dir.clone())
    }

    /// Spec 139 R1: the unattended learn timer (see [`learn_loop`]).
    fn spawn_learn_loop(self: &Arc<Self>) -> Option<tokio::task::JoinHandle<()>> {
        let period = self.learn_interval?;
        self.engine.learn_traj_root()?;
        Some(learn_loop::spawn(
            Arc::clone(&self.engine),
            self.ctx.runtime_dir.clone(),
            period,
        ))
    }

    fn spawn_reactive_loop(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let engine = Arc::clone(&self.engine);
        let ctx = Arc::clone(&self.ctx);
        let scheduler = Arc::clone(&self.scheduler);
        let rx = engine.bus.subscribe();
        tokio::spawn(reactive_loop(engine, ctx, scheduler, rx))
    }
}

async fn reactive_loop(
    engine: Arc<Engine>,
    ctx: Arc<DaemonContext>,
    scheduler: Arc<scheduler::Scheduler>,
    mut rx: tokio::sync::broadcast::Receiver<pii_proto::run::SeqEvent>,
) {
    loop {
        match rx.recv().await {
            Ok(se) => react(&engine, &ctx, &scheduler, &se.event),
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                // Never drop silently (F4): report the gap, then recover the
                // control state the missed events would have applied.
                lag::log_warn(&format!(
                    "reactive loop lagged, {n} events dropped; rescanning suspended and finished runs"
                ));
                lag::recover_suspended(&engine, &scheduler);
                lag::recover_finished(&engine, &ctx);
            }
            Err(_) => break,
        }
    }
}

/// Resolves the channels fs-scope root under `runtime_dir` (Phase 5.0 D2:
/// never a literal path) and ensures the directory exists. A creation
/// failure is a loud config error at daemon start, not a silent per-poll
/// `Denied` discovered later when a channel actually tries to write.
fn resolve_channels_scope(runtime_dir: &Path) -> Result<PathBuf, PiiError> {
    let channels_scope = runtime_dir.join("channels");
    std::fs::create_dir_all(&channels_scope).map_err(|e| {
        PiiError::Invalid(format!(
            "channels scope dir '{}': {e}",
            channels_scope.display()
        ))
    })?;
    Ok(channels_scope)
}

/// Populates the engine-owned worker pool from `[[worker]]` config, each
/// supervisor reporting to the engine's log (spec 108).
fn start_workers(
    engine: &Arc<Engine>,
    cfg: &config::Config,
) -> Arc<Mutex<BTreeMap<String, pii_worker::Supervisor>>> {
    // Populate the engine-owned worker pool (F3.7 worker node + M3's
    // non-test caller for Supervisor::invoke). The node factory holds
    // the same Arc, so workers configured here are reachable from graphs.
    let workers = Arc::clone(&engine.workers);
    for w in &cfg.worker {
        let spec = pii_worker::WorkerSpec {
            name: w.name.clone(),
            program: PathBuf::from(&w.program),
            args: w.args.clone(),
            env: w.env.clone(),
            max_in_flight: w.max_in_flight,
            ready_timeout_ms: 0,
            restart_backoff_ms: 0,
            restart_window_ms: 0,
            max_restarts_in_window: 0,
        };
        let sup = pii_worker::Supervisor::new(spec);
        sup.set_observer(engine.log());
        // Best-effort: a worker that fails to start is visible as
        // `Failed` on `worker.list`, not a fatal daemon-start error.
        let _ = sup.start(&spawn_ctx());
        workers.lock().insert(w.name.clone(), sup);
    }
    workers
}

/// Builds the configured channel, wrapped so its poll/send outcomes report to
/// `log` (spec 108).
fn build_channel(
    c: &config::ChannelSection,
    net: &Arc<pii_net::NetService>,
    grants: &config::ConfigGrants,
    runtime_dir: &Path,
    engine: &Engine,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<Arc<dyn pii_channels::Channel>, PiiError> {
    let log = engine.log();
    let inner = channel_wiring::build_raw_channel(c, net, runtime_dir, &log, env)?;
    // Spec 112 R5: a channel that cannot work says so at startup, by name.
    if let Some(issue) = inner.health_issue() {
        log.event(
            &pii_observe::Event::new(
                "channel",
                pii_observe::Level::Warn,
                pii_observe::names::CHANNEL_MISCONFIGURED,
            )
            .field("channel", c.name.as_str())
            .field("kind", c.kind.as_str())
            .field("issue", issue),
        );
    }
    let channel: Arc<dyn pii_channels::Channel> =
        Arc::new(pii_channels::observe::ObservedChannel::new(inner, log));
    check_sanctioned(c, channel.as_ref(), grants)?;
    // Spec 128: the `post` node reaches the channel through the hub, with the router's own context.
    // The channel's own context says what it may do; the egress itself is allowed by the net
    // grant, which is what the network layer checks (spec 132): the channel's declared hosts,
    // for the channel's principal only. A channel that declares none gets none.
    let rctx = router::channel_ctx(
        &resolve_channels_scope(runtime_dir)?,
        &c.name,
        &channel.egress_hosts(),
    );
    grants.allow_declared(&rctx.principal_str(), &channel.egress_hosts())?;
    engine.channel_hub.insert(Arc::clone(&channel), rctx);
    Ok(channel)
}

/// Spec 137: a host a channel took from its environment is registered only if the configuration
/// sanctions it — `[net] grants` or the channel's own `hosts`. Refused at startup, by name.
fn check_sanctioned(
    c: &config::ChannelSection,
    channel: &dyn pii_channels::Channel,
    grants: &config::ConfigGrants,
) -> Result<(), PiiError> {
    for host in channel.env_derived_hosts() {
        if !c.hosts.contains(&host) && !grants.net_covers(&host) {
            return Err(PiiError::Invalid(format!(
                "channel '{}': egress host '{host}' comes from the environment and the configuration does \
                 not sanction it; add it to `hosts` on [[channel]] '{}' or to `grants` under [net]",
                c.name, c.name
            )));
        }
    }
    Ok(())
}

/// Root capability set for daemon-internal spawns (worker start/reload):
/// `PROC | SPAWN`, unrestricted argv0 (the config already named the
/// program; a second scope here would just duplicate that check).
fn spawn_ctx() -> pii_core::Ctx {
    let mut caps = pii_core::CapSet::empty();
    caps.kinds = pii_core::CapKind::PROC | pii_core::CapKind::SPAWN;
    caps.proc_argv0 = pii_core::ArgvScope::new(vec![String::new()]);
    pii_core::Ctx::root(caps, pii_core::Budget::unlimited())
}

/// Re-reads the config file and applies it to channels and workers (F5.12);
/// never restarts the process. Returns the number of workers reloaded.
///
/// # Errors
/// [`PiiError::Invalid`] if the config file cannot be parsed.
pub fn reload_config(ctx: &DaemonContext) -> Result<u32, PiiError> {
    let cfg = config::Config::load(&ctx.config_path)?;
    let mut reloaded = 0u32;
    {
        let workers = ctx.workers.lock();
        for w in &cfg.worker {
            if let Some(sup) = workers.get(&w.name)
                && sup.reload(&spawn_ctx()).is_ok()
            {
                reloaded += 1;
            }
        }
    }
    let mut channels = ctx.channels.lock();
    for c in &cfg.channel {
        if let Some(rt) = channels.get_mut(&c.name) {
            rt.poll_interval_ms = c.poll_interval_ms;
            rt.graph.clone_from(&c.graph);
        }
    }
    Ok(reloaded)
}

fn react(
    engine: &Arc<Engine>,
    ctx: &Arc<DaemonContext>,
    scheduler: &Arc<scheduler::Scheduler>,
    event: &Event,
) {
    match event {
        Event::RunSuspended { run_id, key, .. } => suspend::spawn(engine, scheduler, run_id, key),
        Event::RunFinished { run_id, terminal } => on_finished(engine, ctx, run_id, terminal),
        _ => {}
    }
}

/// Minimum delay before a finished cron run is re-armed: a graph that finishes at
/// once (or a start that keeps failing) must not re-arm in a hot loop (spec 177).
const CRON_MIN_REARM: std::time::Duration = std::time::Duration::from_secs(1);

/// Starts one cron firing; a refusal is a `cron.start.refused` event (spec 177).
fn start_cron(engine: &Arc<Engine>, ctx: &Arc<DaemonContext>, e: &CronEntry) {
    match engine.start_run(&e.graph, &e.input) {
        Ok(run_id) => {
            ctx.cron_runs
                .lock()
                .insert(run_id.to_string(), e.name.clone());
        }
        Err(err) => engine.log().event(
            &pii_observe::Event::new(
                "cron",
                pii_observe::Level::Error,
                pii_observe::names::CRON_START_REFUSED,
            )
            .field("cron", e.name.as_str())
            .field("graph", e.graph.as_str())
            .field("error", err.to_string()),
        ),
    }
}

/// Re-arms a cron entry whose run finished, after [`CRON_MIN_REARM`].
fn rearm_cron(engine: &Arc<Engine>, ctx: &Arc<DaemonContext>, entry: CronEntry) {
    let (engine, ctx) = (Arc::clone(engine), Arc::clone(ctx));
    // Spec 178: the re-arm thread has no runtime of its own; enter the reactive loop's.
    let rt = tokio::runtime::Handle::current();
    std::thread::spawn(move || {
        let _guard = rt.enter();
        std::thread::sleep(CRON_MIN_REARM);
        start_cron(&engine, &ctx, &entry);
    });
}

/// Reacts to a terminal run (F5.8/F6.5): starts the next cron firing, or
/// replies on the channel that started this run.
fn on_finished(
    engine: &Arc<Engine>,
    ctx: &Arc<DaemonContext>,
    run_id: &str,
    terminal: &WireTerminal,
) {
    // Spec 179: a run parked on its timer journals `Finished{Suspended}`; it is not over, and
    // re-arming here started a new run every CRON_MIN_REARM. Stay tracked until it truly ends.
    let parked = matches!(terminal, WireTerminal::Suspended { .. });
    let cron_name = if parked {
        None
    } else {
        ctx.cron_runs.lock().remove(run_id)
    };
    if let Some(name) = cron_name {
        let entry = ctx.cron.lock().get(&name).cloned();
        if let Some(entry) = entry {
            rearm_cron(engine, ctx, entry);
        }
    }
    let channel_run = ctx.channel_runs.lock().remove(run_id);
    if let Some((channel, chat)) = channel_run {
        let text = match terminal {
            WireTerminal::Success { output } => output
                .get("text")
                .and_then(|v| v.as_str())
                .map_or_else(|| output.to_string(), str::to_owned),
            WireTerminal::Failure { error } => format!("error: {error}"),
            WireTerminal::Suspended { .. } => return,
        };
        // Spec 128: a graph that already answered through a `post` node ends with `text = ""`;
        // an empty reply is never sent (Discord refuses empty content, and it would be noise).
        if text.is_empty() {
            return;
        }
        router::reply(ctx, &channel, &chat, &text);
    }
}

fn write_crash_file(crashes_dir: &Path, info: &std::panic::PanicHookInfo<'_>) {
    let ts_ms = pii_core::SystemClock.now_ms();
    let thread = std::thread::current()
        .name()
        .unwrap_or("unnamed")
        .to_owned();
    let location = info.location().map_or_else(String::new, |l| {
        format!("{}:{}:{}", l.file(), l.line(), l.column())
    });
    let message = panic_message(info);
    let _ = std::fs::create_dir_all(crashes_dir);
    let path = crashes_dir.join(format!("{ts_ms}.json"));
    let payload = serde_json::json!({
        "ts_ms": ts_ms, "thread": thread, "location": location, "message": message,
        "run": Option::<String>::None,
    });
    let _ = std::fs::write(&path, serde_json::to_vec(&payload).unwrap_or_default());
}

fn panic_message(info: &std::panic::PanicHookInfo<'_>) -> String {
    info.payload().downcast_ref::<&str>().map_or_else(
        || {
            info.payload()
                .downcast_ref::<String>()
                .map_or_else(|| String::from("non-string panic payload"), String::clone)
        },
        |s| (*s).to_owned(),
    )
}

/// Cooperative process shutdown (F5.3): the first `SIGTERM`/`SIGINT`
/// triggers graceful shutdown; a second one while shutting down forces an
/// immediate, harder exit (`was_forced()` reports which happened).
#[derive(Default)]
pub struct ShutdownSignal {
    forced: AtomicBool,
}

impl ShutdownSignal {
    /// A fresh, not-yet-forced signal.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `true` once a second signal forced the shutdown (F5.3: exit 130).
    #[must_use]
    pub fn was_forced(&self) -> bool {
        self.forced.load(Ordering::Relaxed)
    }

    async fn one() {
        let term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate());
        let int = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt());
        match (term, int) {
            (Ok(mut t), Ok(mut i)) => {
                tokio::select! { _ = t.recv() => {}, _ = i.recv() => {} }
            }
            _ => std::future::pending::<()>().await,
        }
    }

    /// Waits for the first signal, then races `graceful` against a second
    /// one (F5.3): whichever completes first wins.
    pub async fn wait_then_race<F: Future<Output = ()>>(&self, graceful: F) {
        Self::one().await;
        tokio::select! {
            () = graceful => {}
            () = Self::one() => { self.forced.store(true, Ordering::Relaxed); }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::sync::Arc;

    use pii_abi::{NodeId, ResumeKey, RunEvent, RunId, RunStatus, SuspendReason};
    use pii_core::FixedClock;
    use pii_proto::event::Event;
    use pii_store::timer::TimerWheel;

    use super::reactive_loop;
    use super::scheduler::Scheduler;
    use super::status::DaemonContext;
    use crate::{Engine, EngineConfig};

    pub(super) fn test_ctx(dir: &std::path::Path) -> Arc<DaemonContext> {
        Arc::new(DaemonContext {
            net: Arc::new(pii_net::NetService::new(pii_net::NetConfig {
                resolver: Arc::new(pii_net::SystemResolver),
                grants: Arc::new(pii_net::grant::StaticGrants::default()),
                clock: Arc::new(pii_core::SystemClock),
            })),
            workers: Arc::new(parking_lot::Mutex::new(std::collections::BTreeMap::new())),
            channels: parking_lot::Mutex::new(std::collections::BTreeMap::new()),
            channel_store: Arc::new(
                pii_store::channel::ChannelStore::open(&dir.join("channel.redb")).unwrap(),
            ),
            timer_store: Arc::new(TimerWheel::open(&dir.join("ctx-timer.redb")).unwrap()),
            cron: parking_lot::Mutex::new(std::collections::BTreeMap::new()),
            cron_runs: parking_lot::Mutex::new(std::collections::BTreeMap::new()),
            channel_runs: parking_lot::Mutex::new(std::collections::BTreeMap::new()),
            crashes_dir: dir.join("crashes"),
            config_path: dir.join("config.toml"),
            channels_scope: dir.join("channels"),
            runtime_dir: dir.to_path_buf(),
        })
    }

    /// Spec 125 R1: a graph quarantined by a promotion in effect is refused by
    /// the engine with the named reason; an unrelated graph still runs, and the
    /// same graph runs again once the promotion is reverted.
    // reason: one fixture-then-assert scenario; splitting it would hide the order.
    #[allow(clippy::too_many_lines)]
    #[tokio::test]
    async fn a_quarantined_graph_is_refused_by_the_engine_and_runs_again_after_revert() {
        const EVENT_POLLS: usize = 6000;
        use pii_learn::{
            Candidate, Evidence, Kind, PromoteConfig, PromoteRequest, Promoter, Verdict,
            VerdictRecord, put_verdict, store,
        };
        let tmp_root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../_tmp"));
        std::fs::create_dir_all(tmp_root).unwrap();
        let tmp = tempfile::Builder::new()
            .prefix("quarantine-")
            .tempdir_in(tmp_root)
            .unwrap();
        let dir = tmp.path().to_path_buf();
        let fixture = dir.join("fixture.json");
        std::fs::write(&fixture, r#"{"turns":[]}"#).unwrap();
        let engine = Engine::new(EngineConfig {
            workspace: dir.clone(),
            store_path: dir.join("store.redb"),
            model: format!("scripted:{}", fixture.display()),
            sandbox: pii_tools::sandbox::SandboxProfileName::Off,
            graph_dir: dir.clone(),
            route_policy: std::collections::BTreeMap::new(),
            tiers: Vec::new(),
            session_runs_cap: crate::DEFAULT_SESSION_RUNS_CAP,
            llama_host: None,
        })
        .unwrap();
        engine.attach_daemon_ctx(test_ctx(&dir));
        let rec = Arc::new(pii_observe::Recorder::new());
        engine.attach_log(rec.clone());

        let c = Candidate::new(
            Kind::StuckLoop,
            String::from("graph sick failed in all 3 runs"),
            String::from("stuck_loop:sick:n:other"),
            Evidence {
                runs: vec![String::from("r1")],
                paths: vec![String::from("trajectories/r1.jsonl")],
                counts: std::collections::BTreeMap::from([(String::from("runs"), 3)]),
            },
        )
        .unwrap();
        store::put_candidates(&dir, std::slice::from_ref(&c)).unwrap();
        put_verdict(
            &dir,
            &VerdictRecord {
                candidate: c.id.clone(),
                verdict: Verdict::Confirmed,
                reason: None,
                detail: String::new(),
                shadow: String::new(),
                shadow_runs: Vec::new(),
                numbers: std::collections::BTreeMap::new(),
                evidence_used: Vec::new(),
                missing: Vec::new(),
                bound: None,
                elapsed_ms: 0,
            },
        )
        .unwrap();
        let promoter = Promoter::new(&dir, PromoteConfig::default(), rec.clone());
        let now = crate::engine::runs::learn_now_secs();
        let promo = promoter
            .promote(
                &PromoteRequest {
                    candidate: c.id,
                    kind: String::from("quarantine_graph"),
                    actor: String::from("test"),
                    metric: Some(String::from("graph_definition_changed")),
                    window_secs: Some(3_600),
                    ttl_secs: None,
                },
                now,
            )
            .unwrap();

        let graph = |name: &str| {
            pii_graph::toml_load::load_toml(&format!(
                "name = \"{name}\"\nentry = \"a\"\ncaps = []\n[[node]]\nid = \"a\"\nkind = \"transform\"\nexpr = \"$.input\"\n"
            ))
            .unwrap()
        };
        let start = |name: &str| {
            let id = engine.store.next_run_id().unwrap();
            crate::engine::runs::spawn_run(
                Arc::clone(&engine),
                graph(name),
                id.clone(),
                String::new(),
                String::from("s"),
                None,
            )
            .unwrap();
            id
        };
        // Condition wait, not a sleep. Assumption: a spawned run reaches a terminal status, and
        // under the full parallel workspace suite (and under coverage instrumentation) that can take
        // seconds — the old 2 s bound panicked here as "run {id} never finished" in a fresh-gate run
        // (2026-10-10, spec 119's class: a timing-sensitive test that does not state its bound).
        let finished = |id: &RunId| {
            for _ in 0..6000 {
                if let Ok(Some(h)) = engine.store.get_run(id)
                    && h.status != RunStatus::Running
                {
                    return h;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            panic!("run {id} never finished within 60s");
        };

        // The store status going terminal does not mean the observer has seen the event yet; wait on
        // the event itself. Bound: observer delivery lags the commit by scheduling delay only, so the
        // same 60 s ceiling as `finished` applies (polled every 10 ms).
        let refusals_seen = |at_least: usize| {
            for _ in 0..EVENT_POLLS {
                let ev = rec.events();
                let n = ev
                    .iter()
                    .filter(|e| e.kind == pii_observe::names::RUN_START_REFUSED)
                    .count();
                if n >= at_least {
                    return ev;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            panic!(
                "run.start.refused never reached the observer: {} events seen",
                rec.events().len()
            );
        };

        let refused = start("sick");
        assert_eq!(finished(&refused).status, RunStatus::Failed);
        let want = format!("learn: graph sick is quarantined by promotion {}", promo.id);
        let ev = refusals_seen(1);
        let e = ev
            .iter()
            .find(|e| e.kind == pii_observe::names::RUN_START_REFUSED)
            .expect("run.start.refused");
        assert!(
            e.fields
                .get("error")
                .is_some_and(|v| v.to_string().contains(&want)),
            "{e:?}"
        );

        assert_ne!(
            finished(&start("healthy")).status,
            RunStatus::Failed,
            "an unrelated graph is untouched"
        );

        promoter.revert(&promo.id, "test", now + 1).unwrap();
        assert_ne!(
            finished(&start("sick")).status,
            RunStatus::Failed,
            "the reverted graph runs again"
        );
        assert_eq!(
            refusals_seen(1)
                .iter()
                .filter(|e| e.kind == pii_observe::names::RUN_START_REFUSED)
                .count(),
            1
        );
    }

    /// Spec 79 (D6): a lagged receiver must not lose the `RunSuspended` it
    /// missed. A run is suspended on a timer in the store, the bus is flooded
    /// past its capacity before the loop first reads, and the resume timer
    /// must still end up armed.
    #[tokio::test]
    async fn lagged_control_event_recovers_suspended_timer() {
        let tmp_root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../_tmp"));
        std::fs::create_dir_all(tmp_root).unwrap();
        let tmp = tempfile::Builder::new()
            .prefix("lagged-recover-")
            .tempdir_in(tmp_root)
            .unwrap();
        let dir = tmp.path().to_path_buf();
        let fixture = dir.join("fixture.json");
        std::fs::write(&fixture, r#"{"turns":[]}"#).unwrap();
        let engine = Engine::new(EngineConfig {
            workspace: dir.clone(),
            store_path: dir.join("store.redb"),
            model: format!("scripted:{}", fixture.display()),
            sandbox: pii_tools::sandbox::SandboxProfileName::Off,
            graph_dir: dir.clone(),
            route_policy: std::collections::BTreeMap::new(),
            tiers: Vec::new(),
            session_runs_cap: crate::DEFAULT_SESSION_RUNS_CAP,
            llama_host: None,
        })
        .unwrap();

        let run = RunId::from("r-lagged");
        let key = "timer:r-lagged:t:600000";
        let mut header = pii_abi::RunHeader {
            run: run.clone(),
            graph: String::from("default"),
            status: RunStatus::Suspended,
            seq: 0,
            dur_ms: 0,
        };
        engine
            .store
            .append_event(
                &mut header,
                &RunEvent::Suspended {
                    node: NodeId::from("t"),
                    reason: SuspendReason::Timer { in_ms: 600_000 },
                    key: ResumeKey(String::from(key)),
                },
            )
            .unwrap();

        let ctx = test_ctx(&dir);
        let wheel = Arc::new(TimerWheel::open(&dir.join("timer.redb")).unwrap());
        let scheduler = Scheduler::new(
            Arc::clone(&engine),
            Arc::clone(&wheel),
            Arc::new(FixedClock::new()),
        );

        // Subscribe, then flood past the 4096 capacity before the loop runs:
        // its first recv is `Lagged`, and the RunSuspended is long gone.
        let rx = engine.bus.subscribe();
        for i in 0..5_000u64 {
            engine.bus.broadcast(Event::Lagged {
                sub_id: String::from("burst"),
                dropped: i,
            });
        }
        let handle = tokio::spawn(reactive_loop(Arc::clone(&engine), ctx, scheduler, rx));

        let mut armed = None;
        for _ in 0..100 {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            armed = wheel.next_timer().unwrap();
            if armed.is_some() {
                break;
            }
        }
        handle.abort();
        assert_eq!(
            armed,
            Some(600_000),
            "suspended run's timer must be re-armed after a lag"
        );
    }
}
