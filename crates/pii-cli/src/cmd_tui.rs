//! `pii tui [--connect auto|loopback|<socket>]` (F5.5, F11 opus review):
//! launches the terminal UI over the loopback transport (or a live daemon
//! socket). The daemon is never auto-started.

use crate::cli_util::fail;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;

/// `pii tui [--connect auto|loopback|<socket>] [--workspace D]`.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn cmd_tui(args: &mut std::env::ArgsOs) -> ExitCode {
    let mut connect: Option<String> = None;
    let mut workspace = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    while let Some(flag) = args.next() {
        let flag = flag.to_str().unwrap_or_default().to_owned();
        match flag.as_str() {
            "--connect" => {
                connect = args.next().and_then(|a| a.into_string().ok());
            }
            "--workspace" => {
                if let Some(v) = args.next().and_then(|a| a.into_string().ok()) {
                    workspace = PathBuf::from(v);
                }
            }
            other => {
                fail("tui", format!("unknown flag '{other}'"));
                return ExitCode::from(2);
            }
        }
    }

    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            fail("tui", format!("tokio runtime: {e}"));
            return ExitCode::from(2);
        }
    };
    rt.block_on(run_tui(connect, workspace))
}

async fn run_tui(connect: Option<String>, workspace: PathBuf) -> ExitCode {
    // A live daemon socket wins; otherwise use the in-process loopback.
    let socket = match connect.as_deref() {
        Some("loopback") => None,
        Some("auto") | None => pii_uds::runtime_dir("default")
            .map(|d| d.join("control.sock"))
            .filter(|s| s.exists()),
        Some(path) => Some(PathBuf::from(path)),
    };

    // Spec 108: the TUI owns the terminal, so its failure events are held
    // and written to stderr only after it has restored the terminal.
    let rec = std::sync::Arc::new(pii_observe::Recorder::new());
    let result = if let Some(sock) = socket {
        // Socket opening is centralized in pii-server::uds (net-check
        // allowlist); the TUI only consumes the already-open stream.
        let stream = match pii_uds::client_connect(&sock).await {
            Ok(s) => s,
            Err(e) => {
                fail("tui", &e);
                return ExitCode::from(1);
            }
        };
        let (r, w) = tokio::io::split(stream);
        pii_tui::run_tui_observed(r, w, rec.as_ref()).await
    } else {
        let store_dir = workspace.join(".pii");
        let _ = std::fs::create_dir_all(&store_dir);
        let cfg = EngineConfig {
            workspace: workspace.clone(),
            store_path: store_dir.join("store.redb"),
            model: pii_core::env_var("PII_MODEL")
                .or_else(|| {
                    pii_server::daemon::config::Config::load_from_default_path()
                        .ok()
                        .and_then(|c| c.core.model)
                })
                .unwrap_or_else(|| String::from("scripted:")),
            sandbox: SandboxProfileName::Off,
            graph_dir: store_dir.join("graphs"),
            route_policy: std::collections::BTreeMap::new(),
            session_runs_cap: pii_server::DEFAULT_SESSION_RUNS_CAP,
        };
        let engine = match Engine::new(cfg) {
            Ok(e) => e,
            Err(e) => {
                fail("tui", &e);
                return ExitCode::from(1);
            }
        };
        let stream = pii_server::engine::loopback::spawn_loopback(engine);
        let (r, w) = tokio::io::split(stream);
        pii_tui::run_tui_observed(r, w, rec.as_ref()).await
    };

    for e in rec.events() {
        let fields = serde_json::to_string(&e.fields).unwrap_or_default();
        // reason: nowhere to report a failed stderr write.
        let _ = writeln!(std::io::stderr(), "{}: {} {fields}", e.subsystem, e.kind);
    }
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            fail("tui", &e);
            ExitCode::from(1)
        }
    }
}
