//! `pii tui [--connect auto|loopback|<socket>]` (F5.5, F11 opus review):
//! launches the terminal UI over the loopback transport (or a live daemon
//! socket). The daemon is never auto-started.

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
                eprintln!("error: unknown flag '{other}'");
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
            eprintln!("error: tokio runtime: {e}");
            return ExitCode::from(2);
        }
    };
    rt.block_on(run_tui(connect, workspace))
}

async fn run_tui(connect: Option<String>, workspace: PathBuf) -> ExitCode {
    // A live daemon socket wins; otherwise use the in-process loopback.
    let socket = match connect.as_deref() {
        Some("loopback") => None,
        Some("auto") | None => pii_server::uds::runtime_dir("default")
            .map(|d| d.join("control.sock"))
            .filter(|s| s.exists()),
        Some(path) => Some(PathBuf::from(path)),
    };

    let result = if let Some(sock) = socket {
        let stream = match tokio::net::UnixStream::connect(&sock).await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("error: connect {}: {e}", sock.display());
                return ExitCode::from(1);
            }
        };
        let (r, w) = tokio::io::split(stream);
        pii_tui::run_tui(r, w).await
    } else {
        let store_dir = workspace.join(".pii");
        let _ = std::fs::create_dir_all(&store_dir);
        let cfg = EngineConfig {
            workspace: workspace.clone(),
            store_path: store_dir.join("store.redb"),
            model: pii_core::env_var("PII_MODEL").unwrap_or_else(|| String::from("scripted:")),
            sandbox: SandboxProfileName::Off,
            graph_dir: store_dir.join("graphs"),
        };
        let engine = match Engine::new(cfg) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::from(1);
            }
        };
        let stream = pii_server::loopback::spawn_loopback(engine);
        let (r, w) = tokio::io::split(stream);
        pii_tui::run_tui(r, w).await
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(1)
        }
    }
}
