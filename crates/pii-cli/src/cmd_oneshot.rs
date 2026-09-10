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

/// Resolves the model spec for `pii -p` (2026-09-05): `explicit` (`--model`)
/// wins, then `$PII_MODEL`, then `[core].model` in the persistent config
/// file (see [`pii_server::daemon::config::Config::resolve_model`]).
fn resolve_model(explicit: Option<String>) -> Option<String> {
    pii_server::daemon::config::Config::load_from_default_path()
        .ok()
        .unwrap_or_default()
        .resolve_model(explicit)
}

/// Parsed `pii -p` flags.
struct OneshotArgs {
    prompt: String,
    json: bool,
    model: Option<String>,
    graph: Option<String>,
    workspace: PathBuf,
    sandbox: SandboxProfileName,
    connect: Option<String>,
    /// Extra graph input (F5, opus review): `--input '{"test_command":..}'`
    /// is merged into the run input so verify-fix graphs get their params.
    input: serde_json::Value,
}

/// `pii -p "<prompt>" [--json] [--model M] [--graph G] [--workspace D]
/// [--sandbox off|workspace] [--connect auto|loopback|<socket>]` (F5.5).
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn cmd_oneshot(prompt: String, args: &mut std::env::ArgsOs) -> ExitCode {
    let mut a = OneshotArgs {
        prompt,
        // F31 (opus review): json is the default only when stdout is not a
        // TTY (spec F5.7); on a TTY the human line format is used unless
        // --json is passed.
        json: !std::io::IsTerminal::is_terminal(&std::io::stdout()),
        model: None,
        graph: None,
        workspace: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        sandbox: SandboxProfileName::Off,
        connect: None,
        input: serde_json::Value::Null,
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
            "--input" => {
                let Some(v) = next_owned(args) else {
                    eprintln!("error: --input needs a JSON value");
                    return ExitCode::from(2);
                };
                match serde_json::from_str(&v) {
                    Ok(val) => a.input = val,
                    Err(e) => {
                        eprintln!("error: --input is not valid JSON: {e}");
                        return ExitCode::from(2);
                    }
                }
            }
            other => {
                eprintln!("error: unknown flag '{other}'");
                return ExitCode::from(2);
            }
        }
    }
    let model = resolve_model(a.model.clone());
    let Some(model) = model else {
        eprintln!("no model configured (set PII_MODEL or --model)");
        return ExitCode::from(5);
    };

    // F18 (opus review): apply the sandbox BEFORE building the multi-threaded
    // runtime so every worker thread inherits the Landlock domain. Applying
    // it inside block_on only restricts the calling thread, leaving the
    // spawn_blocking threads that actually run tools unrestricted.
    if !matches!(a.sandbox, SandboxProfileName::Off)
        && let Err(e) =
            pii_tools::sandbox::apply_workspace(&a.workspace, &[], std::path::Path::new("/etc"))
    {
        eprintln!("{e}");
        return ExitCode::from(4);
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
    rt.block_on(run_oneshot(a, model))
}

async fn run_oneshot(a: OneshotArgs, model: String) -> ExitCode {
    let client_info = PeerInfo {
        name: String::from("pii-cli"),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        build: String::from("debug"),
    };

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
        route_policy: std::collections::BTreeMap::new(),
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
    // Socket opening is centralized in pii-server::uds (net-check allowlist);
    // the CLI only consumes the already-open stream.
    let stream = pii_server::uds::client_connect(sock)
        .await
        .map_err(|e| e.to_string())?;
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
        input: if a.input.is_null() {
            None
        } else {
            Some(a.input.clone())
        },
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
    // B5: release the connection before returning (the split reader must
    // not keep the server's per-connection state pinned).
    client.shutdown();
    Ok(exit)
}

fn render_event(ev: &Event, json: bool) {
    if json {
        if let Ok(line) = serde_json::to_string(ev) {
            println!("{line}");
        }
        return;
    }
    if let Some(line) = human_line(ev) {
        println!("{line}");
    }
}

/// Pure formatting for `pii -p`'s human (non `--json`) output — split out
/// from [`render_event`] (2026-09-05) so it is unit-testable without a
/// TTY/live daemon. Returns `None` for events that print nothing.
///
/// # Note on the bug this fixed
/// Before this, `RunFinished`'s `Success` case printed a fixed "done"
/// marker and threw away `output` entirely — the actual model answer was
/// never shown in human mode (`pii -p 'hello'` printed only progress
/// lines). The lack of any test on this path (there was no `#[cfg(test)]`
/// module in this file at all) is exactly how that shipped unnoticed;
/// `human_line`'s tests below cover it.
fn human_line(ev: &Event) -> Option<String> {
    match ev {
        Event::NodeStarted { node, .. } => Some(format!("[{node}] started")),
        Event::ToolCall { tool, .. } => Some(format!("  tool: {tool}")),
        Event::RunFinished { terminal, .. } => match terminal {
            pii_proto::event::WireTerminal::Success { output } => output
                .get("text")
                .and_then(serde_json::Value::as_str)
                .map_or_else(
                    || {
                        if output.is_null() {
                            None
                        } else {
                            Some(output.to_string())
                        }
                    },
                    |text| Some(text.to_owned()),
                ),
            pii_proto::event::WireTerminal::Failure { error } => Some(format!("error: {error}")),
            pii_proto::event::WireTerminal::Suspended { .. } => Some(String::from("(suspended)")),
        },
        Event::Lagged { dropped, .. } => Some(format!("warning: lagged, {dropped} dropped")),
        _ => None,
    }
}

fn next_owned(args: &mut std::env::ArgsOs) -> Option<String> {
    args.next().and_then(|a| a.into_string().ok())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::human_line;
    use pii_proto::event::{Event, WireTerminal};

    #[test]
    fn run_finished_success_prints_the_answer_text() {
        let ev = Event::RunFinished {
            run_id: String::from("r1"),
            terminal: WireTerminal::Success {
                output: serde_json::json!({"text": "6", "turns": 1, "files_changed": []}),
            },
        };
        assert_eq!(human_line(&ev).as_deref(), Some("6"));
    }

    #[test]
    fn run_finished_success_with_no_text_field_prints_raw_json() {
        let ev = Event::RunFinished {
            run_id: String::from("r1"),
            terminal: WireTerminal::Success {
                output: serde_json::json!({"other": 1}),
            },
        };
        assert_eq!(human_line(&ev).as_deref(), Some(r#"{"other":1}"#));
    }

    #[test]
    fn run_finished_success_with_null_output_prints_nothing() {
        let ev = Event::RunFinished {
            run_id: String::from("r1"),
            terminal: WireTerminal::Success {
                output: serde_json::Value::Null,
            },
        };
        assert_eq!(human_line(&ev), None);
    }

    #[test]
    fn run_finished_failure_prints_the_error() {
        let ev = Event::RunFinished {
            run_id: String::from("r1"),
            terminal: WireTerminal::Failure {
                error: String::from("capability denied: net"),
            },
        };
        assert_eq!(
            human_line(&ev).as_deref(),
            Some("error: capability denied: net")
        );
    }

    #[test]
    fn node_started_and_tool_call_print_progress_lines() {
        let started = Event::NodeStarted {
            run_id: String::from("r1"),
            node: String::from("agent"),
        };
        assert_eq!(human_line(&started).as_deref(), Some("[agent] started"));

        let tool = Event::ToolCall {
            run_id: String::from("r1"),
            node: String::from("agent"),
            tool: String::from("fs_read"),
            args: serde_json::Value::Null,
        };
        assert_eq!(human_line(&tool).as_deref(), Some("  tool: fs_read"));
    }
}
