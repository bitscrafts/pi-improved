//! `pii daemon` — binds the UDS control socket and serves connections
//! (F2.8/F2.9, F5.5).

use std::path::PathBuf;
use std::process::ExitCode;

use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;
use tokio::net::UnixListener;

/// `pii daemon [--sandbox off|workspace] [--workspace D] [--store S]`.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn cmd_daemon(args: &mut std::env::ArgsOs) -> ExitCode {
    let mut workspace = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut sandbox = SandboxProfileName::Off;
    let mut store: Option<PathBuf> = None;
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
            other => {
                eprintln!("error: unknown flag '{other}'");
                return ExitCode::from(2);
            }
        }
    }
    // Resolve the runtime dir and store path BEFORE applying the sandbox so
    // the store/runtime dirs can be granted read-write (F17, opus review):
    // a sandboxed daemon must be able to open its own database.
    let Some(runtime_dir) = pii_server::uds::runtime_dir("default") else {
        eprintln!("error: no usable runtime dir (set PII_RUNTIME_DIR)");
        return ExitCode::from(2);
    };
    let store_path = store.unwrap_or_else(|| runtime_dir.join("store.redb"));
    let store_parent = store_path
        .parent()
        .map_or_else(|| PathBuf::from("."), PathBuf::from);

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

    let model = pii_core::env_var("PII_MODEL");
    let Some(model) = model else {
        eprintln!("no model configured (set PII_MODEL)");
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
    rt.block_on(run_daemon(
        workspace,
        sandbox,
        store_path,
        runtime_dir,
        model,
    ))
}

async fn run_daemon(
    workspace: PathBuf,
    sandbox: SandboxProfileName,
    store_path: PathBuf,
    runtime_dir: PathBuf,
    model: String,
) -> ExitCode {
    let sock_path = match pii_server::uds::prepare_socket(&runtime_dir) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };

    let cfg = EngineConfig {
        store_path,
        workspace,
        model,
        sandbox,
        graph_dir: runtime_dir.join("graphs"),
    };
    let engine = match Engine::new(cfg) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };

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
    loop {
        let Ok((stream, _addr)) = listener.accept().await else {
            continue;
        };
        let engine = std::sync::Arc::clone(&engine);
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
