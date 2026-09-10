//! The daemon (F5.1): lifecycle, timer wheel, cron, approvals, channel router.
//!
//! `pii daemon` builds one [`Daemon`] and calls [`Daemon::spawn_background`],
//! then races [`ShutdownSignal`] against its own accept loop.

pub mod approvals;
pub mod config;
pub mod methods;
pub mod router;
pub mod scheduler;
pub mod status;

use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;

use pii_abi::{NodeId, PiiError, RunEvent, RunId, SuspendReason};
use pii_core::Clock as _;
use pii_proto::event::{Event, WireTerminal};

use crate::Engine;
use status::{ChannelRuntime, CronEntry, DaemonContext};

/// Owns everything `pii daemon` starts on top of a bare [`Engine`] (F5.1).
pub struct Daemon {
    /// The shared engine.
    pub engine: Arc<Engine>,
    /// The daemon's runtime context (also attached to `engine`).
    pub ctx: Arc<DaemonContext>,
    /// The timer scheduler.
    pub scheduler: Arc<scheduler::Scheduler>,
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
        let grants = config::ConfigGrants::from_section(&cfg.net)?;
        let net = Arc::new(pii_net::NetService::new(pii_net::NetConfig {
            resolver: Arc::new(pii_net::SystemResolver),
            grants: Arc::new(grants),
            clock: Arc::clone(&clock),
        }));

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
            // Best-effort: a worker that fails to start is visible as
            // `Failed` on `worker.list`, not a fatal daemon-start error.
            let _ = sup.start(&spawn_ctx());
            workers.lock().insert(w.name.clone(), sup);
        }

        let mut channels = BTreeMap::new();
        for c in &cfg.channel {
            let channel = build_channel(
                c,
                Arc::clone(&net) as Arc<dyn pii_core::HttpTransport>,
                runtime_dir,
            )?;
            channels.insert(
                c.name.clone(),
                ChannelRuntime {
                    channel,
                    poll_interval_ms: c.poll_interval_ms,
                    graph: c.graph.clone(),
                },
            );
        }

        let mut cron = BTreeMap::new();
        for c in &cfg.cron {
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

        let ctx = Arc::new(DaemonContext {
            net,
            workers,
            channels: Mutex::new(channels),
            channel_store,
            cron: Mutex::new(cron),
            cron_runs: Mutex::new(BTreeMap::new()),
            channel_runs: Mutex::new(BTreeMap::new()),
            crashes_dir: runtime_dir.join("crashes"),
            config_path,
        });
        engine.attach_daemon_ctx(Arc::clone(&ctx));
        // Spec 32 R1: give the `spawn` node the engine handle it needs to
        // launch child runs (the registry was built inside `Engine::new`,
        // before the Arc existed).
        engine.attach_spawn_engine(Arc::clone(&engine));

        let scheduler = scheduler::Scheduler::new(Arc::clone(&engine), wheel, clock);
        Ok(Arc::new(Self {
            engine,
            ctx,
            scheduler,
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
            if let Ok(run_id) = self.engine.start_run(&e.graph, &e.input) {
                self.ctx
                    .cron_runs
                    .lock()
                    .insert(run_id.to_string(), e.name.clone());
            }
        }
    }

    /// Spawns every background subsystem (F5.1): the scheduler loop, one
    /// poller per channel, and the reactive suspend/finish handler. Returns
    /// their join handles so the caller can abort them during shutdown.
    pub fn spawn_background(self: &Arc<Self>) -> Vec<tokio::task::JoinHandle<()>> {
        let mut handles = vec![tokio::spawn(Arc::clone(&self.scheduler).run())];
        handles.extend(router::spawn_all(&self.engine, &self.ctx));
        handles.push(self.spawn_reactive_loop());
        handles
    }

    fn spawn_reactive_loop(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let engine = Arc::clone(&self.engine);
        let ctx = Arc::clone(&self.ctx);
        let scheduler = Arc::clone(&self.scheduler);
        let mut rx = engine.bus.subscribe();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(se) => react(&engine, &ctx, &scheduler, &se.event),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(_) => break,
                }
            }
        })
    }
}

fn build_channel(
    c: &config::ChannelSection,
    transport: Arc<dyn pii_core::HttpTransport>,
    runtime_dir: &Path,
) -> Result<Arc<dyn pii_channels::Channel>, PiiError> {
    match c.kind.as_str() {
        "discord" => Ok(Arc::new(pii_channels::discord::DiscordChannel::from_env(
            &c.name, transport,
        )?)),
        "cli" => {
            let inbox = c.inbox.clone().map_or_else(
                || runtime_dir.join(format!("{}.in.jsonl", c.name)),
                PathBuf::from,
            );
            let outbox = c.outbox.clone().map_or_else(
                || runtime_dir.join(format!("{}.out.jsonl", c.name)),
                PathBuf::from,
            );
            Ok(Arc::new(pii_channels::cli::CliChannel::new(
                c.name.clone(),
                inbox,
                outbox,
            )))
        }
        other => Err(PiiError::Invalid(format!(
            "channel '{}': unknown kind '{other}'",
            c.name
        ))),
    }
}

/// Root capability set for daemon-internal spawns (worker start/reload):
/// `PROC | SPAWN`, unrestricted argv0 (the config already named the
/// program; a second scope here would just duplicate that check).
fn spawn_ctx() -> pii_core::Ctx {
    let mut caps = pii_core::CapSet::empty();
    caps.kinds = pii_core::CapKind::PROC | pii_core::CapKind::SPAWN;
    caps.proc_argv0 = pii_core::StringScope::new(vec![String::new()]);
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
        Event::RunSuspended { run_id, key, .. } => on_suspended(engine, scheduler, run_id, key),
        Event::RunFinished { run_id, terminal } => on_finished(engine, ctx, run_id, terminal),
        _ => {}
    }
}

/// Reacts to a suspension (F5.5/F5.9): arms the timer wheel or creates the
/// pending approval record, read back from the journal for the full
/// structured [`SuspendReason`] (the wire event only carries a display
/// string).
fn on_suspended(
    engine: &Arc<Engine>,
    scheduler: &Arc<scheduler::Scheduler>,
    run_id: &str,
    key: &str,
) {
    let Ok(journal) = engine.store.read_journal(&RunId::from(run_id)) else {
        return;
    };
    let found = journal.iter().rev().find_map(|(_, e)| match e {
        RunEvent::Suspended {
            node,
            reason,
            key: k,
        } if k.0 == key => Some((node, reason)),
        _ => None,
    });
    let Some((node, reason)): Option<(&NodeId, &SuspendReason)> = found else {
        return;
    };
    match reason {
        SuspendReason::Timer { .. } => {
            if let Some(fire_at) = key.rsplit(':').next().and_then(|s| s.parse::<u64>().ok()) {
                scheduler.arm(fire_at, run_id, key);
            }
        }
        SuspendReason::Approval { policy, prompt } => {
            let approval_id = key.strip_prefix("approval:").unwrap_or(key);
            let _ = approvals::request(engine, approval_id, run_id, node.as_str(), policy, prompt);
        }
        SuspendReason::Event { .. } | SuspendReason::HumanInput { .. } => {}
    }
}

/// Reacts to a terminal run (F5.8/F6.5): starts the next cron firing, or
/// replies on the channel that started this run.
fn on_finished(
    engine: &Arc<Engine>,
    ctx: &Arc<DaemonContext>,
    run_id: &str,
    terminal: &WireTerminal,
) {
    let cron_name = ctx.cron_runs.lock().remove(run_id);
    if let Some(name) = cron_name {
        let entry = ctx.cron.lock().get(&name).cloned();
        if let Some(entry) = entry
            && let Ok(new_run) = engine.start_run(&entry.graph, &entry.input)
        {
            ctx.cron_runs.lock().insert(new_run.to_string(), name);
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
