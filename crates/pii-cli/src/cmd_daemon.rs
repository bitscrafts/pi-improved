//! `pii daemon` — binds the UDS control socket, starts the daemon
//! subsystems, and serves connections (F2.8/F2.9, F5.1-F5.5).

use std::path::PathBuf;
use std::process::ExitCode;

use pii_server::daemon::{Daemon, ShutdownSignal, config::Config};
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;
use tokio::net::UnixListener;

/// `pii daemon [--config P] [--sandbox off|workspace] [--workspace D] [--store S]`.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
#[allow(clippy::too_many_lines)] // reason: flag parsing + sandbox + config
// resolution is one cohesive startup sequence (pre-existing pattern in this
// file); splitting it would scatter the ordering invariants across helpers.
pub(crate) fn cmd_daemon(args: &mut std::env::ArgsOs) -> ExitCode {
    let mut workspace = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut sandbox = SandboxProfileName::Off;
    let mut store: Option<PathBuf> = None;
    let mut config: Option<PathBuf> = None;
    while let Some(flag) = args.next() {
        let flag = flag.to_str().unwrap_or_default().to_owned();
        match flag.as_str() {
            "--workspace" => {
                if let Some(v) = args.next().and_then(|a| a.into_string().ok()) {
                    workspace = PathBuf::from(v);
                }
            }
            "--store" => {
                store = args
                    .next()
                    .and_then(|a| a.into_string().ok())
                    .map(PathBuf::from);
            }
            "--config" => {
                config = args
                    .next()
                    .and_then(|a| a.into_string().ok())
                    .map(PathBuf::from);
            }
            "--sandbox" => {
                let Some(v) = args.next().and_then(|a| a.into_string().ok()) else {
                    eprintln!("error: --sandbox needs a value");
                    return ExitCode::from(2);
                };
                match pii_tools::sandbox::parse_profile(&v) {
                    Ok(p) => sandbox = p,
                    Err(e) => {
                        eprintln!("error: {e}");
                        return ExitCode::from(2);
                    }
                }
            }
            // P4.5 (spec 18 R1): explicit no-op — the daemon is already
            // foreground. The flag documents the supervisor contract and
            // future-proofs a daemonize path; it does not change behavior.
            "--foreground" => {}
            other => {
                eprintln!("error: unknown flag '{other}'");
                return ExitCode::from(2);
            }
        }
    }
    // Resolve the runtime dir and store path BEFORE applying the sandbox so
    // the store/runtime dirs can be granted read-write (F17, opus review):
    // a sandboxed daemon must be able to open its own database.
    let Some(runtime_dir) = pii_uds::runtime_dir("default") else {
        eprintln!("error: no usable runtime dir (set PII_RUNTIME_DIR)");
        return ExitCode::from(2);
    };
    let store_path = store.unwrap_or_else(|| runtime_dir.join("store.redb"));
    let store_parent = store_path
        .parent()
        .map_or_else(|| PathBuf::from("."), PathBuf::from);
    // 2026-09-05: default `config_path` to the SAME unified file `pii -p`/
    // `pii tui` read (`Config::default_path()`, normally `~/.pii/pii.toml`)
    // rather than `<runtime_dir>/pii.toml` — so a single-instance operator
    // has exactly one config file for everything (core model/runtime_dir
    // PLUS [net]/[[worker]]/[[channel]]/[memory]/[[cron]]), not two
    // similarly-named files at different paths. `--config`/`$PII_CONFIG`
    // still override outright for anyone running multiple instances.
    let config_path = config
        .or_else(|| pii_core::env_var("PII_CONFIG").map(PathBuf::from))
        .unwrap_or_else(Config::default_path);

    // Sandbox enforcement is verified BEFORE the model lookup: a requested
    // profile that cannot be positively enforced must exit 4 regardless of
    // whether a model is configured (F3.10, part-2 EC6).
    if !matches!(sandbox, SandboxProfileName::Off)
        && let Err(e) = pii_tools::sandbox::apply_workspace(
            &workspace,
            &[store_parent, runtime_dir.clone()],
            std::path::Path::new("/etc"),
        )
    {
        eprintln!("{e}");
        return ExitCode::from(4);
    }

    let cfg = if config_path.exists() {
        match Config::load(&config_path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::from(2);
            }
        }
    } else {
        Config::default()
    };

    // NET-cap wiring and the "no model configured" error both need the
    // resolved model before the engine is built (2026-09-05: `[core].model`
    // in the same config file is now a valid third source, after
    // `$PII_MODEL` — see `Config::resolve_model`'s doc comment for the
    // precedence order this restores parity with `pii -p`/`pii tui`).
    let model = cfg.resolve_model(None);
    let Some(model) = model else {
        eprintln!("no model configured (set PII_MODEL, or [core].model in the config file)");
        return ExitCode::from(5);
    };

    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        // RSS-gate fix (2026-09-05): every run.start/resume spawns a fresh
        // OS thread via spawn_blocking (runs.rs). Under a fast, sequential
        // soak (F5.14's rss-gate: --runs 3000) that grows tokio's blocking
        // pool toward its 512-thread default cap — measured 279 threads
        // under load vs 1 idle — and each churned thread's glibc-cached
        // stack/arena shows up as climbing daemon RSS with a FLAT fd count
        // (ruled out as a connection leak; see HANDOFF.md). Capping the
        // pool bounds worst-case thread count; genuinely concurrent
        // channel/cron/soak load queues past the cap rather than spawning
        // unboundedly, which is the correct tradeoff for a daemon whose
        // real workload is bursty, not a sustained high-throughput soak.
        // `worker_threads` also matters: tokio defaults to num_cpus (32 on
        // this host) async workers, which a low-throughput control-plane
        // daemon does not need — 4 is ample headroom for connection I/O.
        .worker_threads(
            pii_core::env_var("PII_DAEMON_WORKER_THREADS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(4),
        )
        .max_blocking_threads(
            pii_core::env_var("PII_DAEMON_MAX_BLOCKING")
                .and_then(|v| v.parse().ok())
                .unwrap_or(8),
        )
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("error: tokio runtime: {e}");
            return ExitCode::from(2);
        }
    };
    rt.block_on(run_daemon(
        workspace,
        sandbox,
        store_path,
        runtime_dir,
        config_path,
        cfg,
        model,
    ))
}

#[allow(clippy::too_many_arguments)]
// reason: every argument is a distinct
// resolved startup value; bundling them into a struct would not shrink this
// one call site, only move the field list elsewhere.
#[allow(clippy::too_many_lines)] // reason: one cohesive daemon-startup
// sequence (engine, daemon context, listener, accept loop, shutdown race);
// splitting it would scatter the ordering invariants across helpers.
async fn run_daemon(
    workspace: PathBuf,
    sandbox: SandboxProfileName,
    store_path: PathBuf,
    runtime_dir: PathBuf,
    config_path: PathBuf,
    cfg: Config,
    model: String,
) -> ExitCode {
    let sock_path = match pii_server::uds::prepare_socket(&runtime_dir) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };

    let engine_cfg = EngineConfig {
        store_path,
        workspace,
        model,
        sandbox,
        graph_dir: runtime_dir.join("graphs"),
        route_policy: cfg.evolve.routing.clone(),
    };
    let engine = match Engine::new(engine_cfg) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    // P4.2 (spec 16): a daemon restart must not be amnesia — re-seed sessions
    // that survived, and mark crash-interrupted runs Failed instead of leaving
    // permanent zombies in `run.list --status running`.
    engine.seed_sessions_from_store();
    engine.reconcile_boot();

    // PII_TEST_CLOCK_MS (test-only): when set, the daemon uses a FixedClock
    // starting at that epoch so scripts can drive timers deterministically.
    let clock: std::sync::Arc<dyn pii_core::Clock> =
        match pii_core::env_var("PII_TEST_CLOCK_MS").and_then(|v| v.parse().ok()) {
            Some(start_ms) => {
                let fc = pii_core::FixedClock::new();
                fc.advance(start_ms);
                std::sync::Arc::new(fc)
            }
            None => std::sync::Arc::new(pii_core::SystemClock),
        };
    let daemon = match Daemon::build(engine.clone(), &cfg, &runtime_dir, config_path, clock) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    daemon.install_panic_hook();
    daemon.start_cron_entries();
    let mut handles = daemon.spawn_background();

    // reason: the daemon binds the UDS control socket; NetService (Phase 3)
    // is not yet wired. The socket dir is chmod 0700 in prepare_socket (F10)
    // and the socket is chmod 0600 immediately after bind, so the mode
    // discipline the ban exists to enforce is honoured here.
    #[allow(clippy::disallowed_methods)]
    let listener = match UnixListener::bind(&sock_path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("error: bind {}: {e}", sock_path.display());
            return ExitCode::from(2);
        }
    };
    let _ = std::fs::set_permissions(&sock_path, std::fs::Permissions::from_mode(0o600));

    let daemon_uid = self_uid();
    let accept_engine = engine.clone();
    let accept = tokio::spawn(async move {
        loop {
            let Ok((stream, _addr)) = listener.accept().await else {
                continue;
            };
            let engine = std::sync::Arc::clone(&accept_engine);
            tokio::spawn(async move {
                // F9 (opus review): peercred must be readable; an unreadable
                // credential is a refusal, never a fail-open grant.
                let Ok(cred) = stream.peer_cred() else {
                    let _ = write_101(stream).await;
                    return;
                };
                let uid = cred.uid();
                if let Err(e) = pii_server::uds::authorize_peer(daemon_uid, uid) {
                    // F2.8/F9: log the refusal AND write a code-101 frame before
                    // closing (EC3 greps for this line).
                    #[allow(clippy::print_stderr)]
                    {
                        eprintln!("refused uid {uid}: {}", e.message);
                    }
                    let _ = write_101(stream).await;
                    return;
                }
                pii_server::conn::serve_connection(
                    engine,
                    stream,
                    pii_server::conn::PeerIdentity::Uds { uid: Some(uid) },
                )
                .await;
            });
        }
    });
    handles.push(accept);

    // F5.3: exit 0 within 2s of the first SIGTERM/SIGINT; a second signal
    // while shutting down aborts immediately (exit 130).
    let shutdown = ShutdownSignal::new();
    let sock_for_cleanup = sock_path.clone();
    shutdown
        .wait_then_race(async move {
            for h in &handles {
                h.abort();
            }
            engine.cancel_all();
            let _ = std::fs::remove_file(&sock_for_cleanup);
        })
        .await;
    if shutdown.was_forced() {
        ExitCode::from(130)
    } else {
        ExitCode::SUCCESS
    }
}

/// Writes a single code-101 `RpcError` frame to a refused peer before the
/// connection is dropped (F9: the peer must see the refusal, not a bare EOF).
async fn write_101(stream: tokio::net::UnixStream) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt;
    let resp = pii_proto::Response::err(
        pii_proto::Id::Num(0),
        pii_proto::RpcError::new(pii_proto::ErrorCode::Unauthorized, "refused"),
    );
    let line = pii_proto::write_frame(&pii_proto::Envelope::Response(resp)).unwrap_or_else(|_| {
        String::from(
            "{\"jsonrpc\":\"2.0\",\"id\":0,\"error\":{\"code\":101,\"message\":\"refused\"}}",
        )
    });
    let mut s = stream;
    s.write_all(line.as_bytes()).await?;
    s.write_all(b"\n").await?;
    s.flush().await
}

fn self_uid() -> u32 {
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        for line in status.lines() {
            if let Some(rest) = line.strip_prefix("Uid:") {
                return rest
                    .split_whitespace()
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
            }
        }
    }
    0
}

use std::os::unix::fs::PermissionsExt;
