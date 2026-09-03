//! `pii -p` headless coding entry point (F5.5/F5.6/F5.7).
//!
//! Transport selection: a live daemon socket wins (UDS); otherwise the
//! engine is constructed in-process and served over the loopback duplex.
//! **The daemon is never auto-started.** One dispatch path either way —
//! `serve_connection` is identical for both (F5.6/F5.8).

use std::path::PathBuf;
use std::process::ExitCode;

use pii_proto::event::{Event, EventFilter};
use pii_proto::method::Method;
use pii_proto::session::{ContentBlock, SessionCreateParams, SessionCreateResult};
use pii_proto::sys::PeerInfo;
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;

/// Parsed `pii -p` flags.
struct OneshotArgs {
    prompt: String,
    json: bool,
    model: Option<String>,
    graph: Option<String>,
    workspace: PathBuf,
    sandbox: SandboxProfileName,
    connect: Option<String>,
}

/// `pii -p "<prompt>" [--json] [--model M] [--graph G] [--workspace D]
/// [--sandbox off|workspace] [--connect auto|loopback|<socket>]` (F5.5).
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn cmd_oneshot(prompt: String, args: &mut std::env::ArgsOs) -> ExitCode {
    let mut a = OneshotArgs {
        prompt,
        json: true,
        model: None,
        graph: None,
        workspace: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        sandbox: SandboxProfileName::Off,
        connect: None,
    };
    while let Some(flag) = args.next() {
        let flag = flag.to_str().unwrap_or_default().to_owned();
        match flag.as_str() {
            "--json" => a.json = true,
            "--model" => a.model = next_owned(args),
            "--graph" => a.graph = next_owned(args),
            "--workspace" => a.workspace = next_owned(args).map_or(a.workspace, PathBuf::from),
            "--sandbox" => {
                let Some(v) = next_owned(args) else {
                    eprintln!("error: --sandbox needs a value");
                    return ExitCode::from(2);
                };
                match pii_tools::sandbox::parse_profile(&v) {
                    Ok(p) => a.sandbox = p,
                    Err(e) => {
                        eprintln!("error: {e}");
                        return ExitCode::from(2);
                    }
                }
            }
            "--connect" => a.connect = next_owned(args),
            other => {
                eprintln!("error: unknown flag '{other}'");
                return ExitCode::from(2);
            }
        }
    }
    let model = a.model.clone().or_else(|| pii_core::env_var("PII_MODEL"));
    let Some(model) = model else {
        eprintln!("no model configured (set PII_MODEL or --model)");
        return ExitCode::from(5);
    };

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
    rt.block_on(run_oneshot(a, model))
}

async fn run_oneshot(a: OneshotArgs, model: String) -> ExitCode {
    let client_info = PeerInfo {
        name: String::from("pii-cli"),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        build: String::from("debug"),
    };

    if !matches!(a.sandbox, SandboxProfileName::Off)
        && let Err(e) =
            pii_tools::sandbox::apply_workspace(&a.workspace, &[], std::path::Path::new("/etc"))
    {
        eprintln!("{e}");
        return ExitCode::from(4);
    }

    // F5.6: a live daemon socket wins; the daemon is never auto-started.
    let socket = match a.connect.as_deref() {
        Some("loopback") => None,
        Some("auto") | None => pii_server::uds::runtime_dir("default")
            .map(|d| d.join("control.sock"))
            .filter(|s| s.exists()),
        Some(path) => Some(PathBuf::from(path)),
    };

    let result = if let Some(sock) = socket {
        run_over_uds(&sock, &a, &client_info, &model).await
    } else {
        run_over_loopback(&a, &client_info, &model).await
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(1)
        }
    }
}

async fn run_over_loopback(
    a: &OneshotArgs,
    client_info: &PeerInfo,
    model: &str,
) -> Result<ExitCode, String> {
    let store_dir = a.workspace.join(".pii");
    std::fs::create_dir_all(&store_dir)
        .map_err(|e| format!("mkdir {}: {e}", store_dir.display()))?;
    let cfg = EngineConfig {
        workspace: a.workspace.clone(),
        store_path: store_dir.join("store.redb"),
        model: model.to_owned(),
        sandbox: a.sandbox,
        graph_dir: store_dir.join("graphs"),
    };
    let engine = Engine::new(cfg).map_err(|e| e.to_string())?;
    let stream = pii_server::loopback::spawn_loopback(engine);
    let (r, w) = tokio::io::split(stream);
    drive_session(r, w, client_info.clone(), a, model).await
}

async fn run_over_uds(
    sock: &std::path::Path,
    a: &OneshotArgs,
    client_info: &PeerInfo,
    model: &str,
) -> Result<ExitCode, String> {
    let stream = tokio::net::UnixStream::connect(sock)
        .await
        .map_err(|e| format!("connect {}: {e}", sock.display()))?;
    let (r, w) = tokio::io::split(stream);
    drive_session(r, w, client_info.clone(), a, model).await
}

async fn drive_session<R, W>(
    r: R,
    w: W,
    client_info: PeerInfo,
    a: &OneshotArgs,
    model: &str,
) -> Result<ExitCode, String>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (client, _hello) = pii_proto::client::Client::connect(r, w, client_info)
        .await
        .map_err(|e| e.to_string())?;
    let mut events = client.events();

    let create_params = serde_json::to_value(SessionCreateParams {
        cwd: a.workspace.display().to_string(),
        model: Some(model.to_owned()),
        tools: Vec::new(),
        graph: a.graph.clone(),
    })
    .map_err(|e| e.to_string())?;
    let created = client
        .call(Method::SessionCreate, create_params)
        .await
        .map_err(|e| e.message)?;
    let created: SessionCreateResult =
        serde_json::from_value(created).map_err(|e| e.to_string())?;

    let prompt_params = serde_json::to_value(pii_proto::session::SessionPromptParams {
        session_id: created.session_id.clone(),
        content: vec![ContentBlock::Text {
            text: a.prompt.clone(),
        }],
    })
    .map_err(|e| e.to_string())?;
    let started = client
        .call(Method::SessionPrompt, prompt_params)
        .await
        .map_err(|e| e.message)?;
    let started: pii_proto::session::SessionPromptResult =
        serde_json::from_value(started).map_err(|e| e.to_string())?;

    let sub_params = serde_json::to_value(EventFilter {
        run: Some(started.run_id.clone()),
        session: None,
        kinds: Vec::new(),
        since_seq: Some(0),
    })
    .map_err(|e| e.to_string())?;
    client
        .call(Method::EventSubscribe, sub_params)
        .await
        .map_err(|e| e.message)?;

    let mut exit = ExitCode::SUCCESS;
    while let Ok(push) = events.recv().await {
        render_event(&push.event, a.json);
        if let Event::RunFinished { terminal, .. } = &push.event {
            if matches!(terminal, pii_proto::event::WireTerminal::Failure { .. }) {
                exit = ExitCode::from(1);
            }
            break;
        }
        if matches!(push.event, Event::RunCancelled { .. }) {
            exit = ExitCode::from(1);
            break;
        }
    }
    Ok(exit)
}

fn render_event(ev: &Event, json: bool) {
    if json {
        if let Ok(line) = serde_json::to_string(ev) {
            println!("{line}");
        }
        return;
    }
    match ev {
        Event::NodeStarted { node, .. } => println!("⏵ {node}"),
        Event::ToolCall { tool, .. } => println!("  ↳ {tool}"),
        Event::RunFinished { terminal, .. } => match terminal {
            pii_proto::event::WireTerminal::Success { .. } => println!("✔ done"),
            pii_proto::event::WireTerminal::Failure { error } => println!("✘ {error}"),
            pii_proto::event::WireTerminal::Suspended { .. } => println!("… suspended"),
        },
        Event::Lagged { dropped, .. } => println!("⚠ lagged: {dropped} dropped"),
        _ => {}
    }
}

fn next_owned(args: &mut std::env::ArgsOs) -> Option<String> {
    args.next().and_then(|a| a.into_string().ok())
}
