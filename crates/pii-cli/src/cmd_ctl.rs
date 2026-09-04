//! Thin protocol-calling CLI subcommands (F7.1): `net`, `worker`, `memory`,
//! `approve`, `cron`. Each connects to the daemon's UDS control socket,
//! calls one method, prints the JSON result, and exits.

use std::process::ExitCode;

use pii_proto::client::Client;
use pii_proto::method::Method;
use pii_proto::sys::PeerInfo;

fn client_info() -> PeerInfo {
    PeerInfo {
        name: String::from("pii"),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        build: String::from("cli"),
    }
}

/// Connects to `<runtime_dir>/control.sock`, calls `method`, and returns
/// the raw JSON result.
async fn call_async(
    method: Method,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let runtime_dir = pii_server::uds::runtime_dir("default")
        .ok_or_else(|| String::from("no usable runtime dir (set PII_RUNTIME_DIR)"))?;
    let sock = runtime_dir.join("control.sock");
    let stream = pii_server::uds::client_connect(&sock)
        .await
        .map_err(|e| e.to_string())?;
    let (r, w) = tokio::io::split(stream);
    let (client, _hello) = Client::connect(r, w, client_info())
        .await
        .map_err(|e| e.to_string())?;
    client
        .call(method, params)
        .await
        .map_err(|e| format!("{}: {}", e.code, e.message))
}

// Shared helpers are used from sibling modules (`cmd_sys`, `cmd_soak`), so the
// crate-wide visibility is load-bearing even though the module is private.
#[allow(clippy::redundant_pub_crate)]
pub(crate) fn run_call(
    method: Method,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    rt.block_on(call_async(method, params))
}

#[allow(clippy::redundant_pub_crate)]
pub(crate) fn print_result(method: Method, params: serde_json::Value) -> ExitCode {
    match run_call(method, params) {
        Ok(v) => {
            println!("{v}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(2)
        }
    }
}

#[allow(clippy::redundant_pub_crate)]
pub(crate) fn next_str(args: &mut std::env::ArgsOs) -> Option<String> {
    args.next().and_then(|a| a.into_string().ok())
}

/// `pii net ls`.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn cmd_net(args: &mut std::env::ArgsOs) -> ExitCode {
    if next_str(args).as_deref() != Some("ls") {
        eprintln!("usage: pii net ls");
        return ExitCode::from(2);
    }
    match run_call(Method::SysStatus, serde_json::json!({})) {
        Ok(v) => {
            println!(
                "{}",
                v.get("listeners").cloned().unwrap_or(serde_json::json!([]))
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(2)
        }
    }
}

/// `pii worker ls|reload <name>`.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn cmd_worker(args: &mut std::env::ArgsOs) -> ExitCode {
    match next_str(args).as_deref() {
        Some("ls") => print_result(Method::WorkerList, serde_json::json!({})),
        Some("reload") => {
            let Some(name) = next_str(args) else {
                eprintln!("usage: pii worker reload <name>");
                return ExitCode::from(2);
            };
            print_result(Method::WorkerReload, serde_json::json!({"name": name}))
        }
        _ => {
            eprintln!("usage: pii worker {{ls|reload <name>}}");
            ExitCode::from(2)
        }
    }
}

/// `pii memory search <q> [--k N] | put <text> [--layer L] [--kind K] | consolidate`.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn cmd_memory(args: &mut std::env::ArgsOs) -> ExitCode {
    match next_str(args).as_deref() {
        Some("search") => {
            let Some(q) = next_str(args) else {
                eprintln!("usage: pii memory search <q> [--k N]");
                return ExitCode::from(2);
            };
            let mut k = 6u64;
            while let Some(a) = next_str(args) {
                if a == "--k" {
                    k = next_str(args).and_then(|s| s.parse().ok()).unwrap_or(6);
                }
            }
            print_result(Method::MemorySearch, serde_json::json!({"q": q, "k": k}))
        }
        Some("put") => {
            let Some(text) = next_str(args) else {
                eprintln!("usage: pii memory put <text> [--layer L] [--kind K]");
                return ExitCode::from(2);
            };
            let mut layer = String::from("l1");
            let mut kind = String::from("atom");
            while let Some(a) = next_str(args) {
                match a.as_str() {
                    "--layer" => layer = next_str(args).unwrap_or(layer),
                    "--kind" => kind = next_str(args).unwrap_or(kind),
                    _ => {}
                }
            }
            print_result(
                Method::MemoryPut,
                serde_json::json!({"text": text, "layer": layer, "kind": kind}),
            )
        }
        Some("consolidate") => print_result(Method::MemoryConsolidate, serde_json::json!({})),
        _ => {
            eprintln!("usage: pii memory {{search|put|consolidate}} ...");
            ExitCode::from(2)
        }
    }
}

/// `pii approve ls|allow <id> [--note N]|deny <id>`.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn cmd_approve(args: &mut std::env::ArgsOs) -> ExitCode {
    match next_str(args).as_deref() {
        Some("ls") => print_result(Method::ApproveList, serde_json::json!({})),
        Some("allow") => resolve(args, true),
        Some("deny") => resolve(args, false),
        _ => {
            eprintln!("usage: pii approve {{ls|allow <id> [--note N]|deny <id>}}");
            ExitCode::from(2)
        }
    }
}

fn resolve(args: &mut std::env::ArgsOs, allow: bool) -> ExitCode {
    let Some(id) = next_str(args) else {
        eprintln!("usage: pii approve {{allow|deny}} <id> [--note N]");
        return ExitCode::from(2);
    };
    let note: Option<String> = match next_str(args) {
        Some(flag) if flag == "--note" => next_str(args),
        _ => None,
    };
    let mut params = serde_json::json!({"id": id, "allow": allow});
    if let Some(n) = note
        && let Some(obj) = params.as_object_mut()
    {
        obj.insert(String::from("note"), serde_json::json!(n));
    }
    print_result(Method::ApproveResolve, params)
}

/// `pii cron ls`.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn cmd_cron(args: &mut std::env::ArgsOs) -> ExitCode {
    if next_str(args).as_deref() == Some("ls") {
        print_result(Method::CronList, serde_json::json!({}))
    } else {
        eprintln!("usage: pii cron ls");
        ExitCode::from(2)
    }
}
