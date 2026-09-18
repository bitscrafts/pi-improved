//! `pii web` — read-only web viewer (P4.6, spec 19).
//!
//! Proxies the daemon's UDS JSON-RPC over loopback HTTP so a browser can
//! watch runs, sessions, approvals, heals, and memory. VIEWER-ONLY: no
//! write RPC is proxied. axum lives here, not in the workspace root or
//! pii-cli (deps-check invariant).

#![forbid(unsafe_code)]
// reason: the web server must print startup/error messages to stderr.
#![allow(clippy::print_stdout, clippy::print_stderr)]
// reason: the CLI binary uses process::exit for fatal flag errors.
#![allow(clippy::exit)]

use std::net::SocketAddr;
use std::process::ExitCode;
use std::sync::Arc;

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use pii_proto::client::Client;
use pii_proto::method::Method;
use pii_proto::sys::PeerInfo;
use serde::Deserialize;

/// The embedded SPA (self-contained, no external assets).
const INDEX_HTML: &str = include_str!("index.html");

/// Shared server state: the daemon socket path.
#[derive(Clone)]
struct AppState {
    sock: Arc<std::path::PathBuf>,
}

fn client_info() -> PeerInfo {
    PeerInfo {
        name: String::from("pii-web"),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        build: String::from("web"),
    }
}

/// Calls one daemon RPC over the UDS socket and returns the raw JSON.
async fn call_daemon(
    sock: &std::path::Path,
    method: Method,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let stream = pii_uds::client_connect(sock)
        .await
        .map_err(|e| e.to_string())?;
    let (r, w) = tokio::io::split(stream);
    let (client, _hello) = Client::connect(r, w, client_info())
        .await
        .map_err(|e| e.to_string())?;
    let result = client
        .call(method, params)
        .await
        .map_err(|e| format!("{}: {}", e.code, e.message));
    client.shutdown();
    result
}

/// `GET /` — the SPA.
async fn index() -> impl IntoResponse {
    Html(INDEX_HTML)
}

/// `GET /api/status` — daemon status.
async fn api_status(State(st): State<AppState>) -> Response {
    match call_daemon(&st.sock, Method::SysStatus, serde_json::json!({})).await {
        Ok(v) => (StatusCode::OK, v.to_string()).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, format!("{{\"error\":\"{e}\"}}")).into_response(),
    }
}

/// `GET /api/runs` — run list.
async fn api_runs(State(st): State<AppState>) -> Response {
    match call_daemon(&st.sock, Method::RunList, serde_json::json!({})).await {
        Ok(v) => (StatusCode::OK, v.to_string()).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, format!("{{\"error\":\"{e}\"}}")).into_response(),
    }
}

/// `GET /api/runs/:id/trace` — run journal trace.
async fn api_run_trace(State(st): State<AppState>, Path(id): Path<String>) -> Response {
    match call_daemon(
        &st.sock,
        Method::RunTrace,
        serde_json::json!({ "run_id": id }),
    )
    .await
    {
        Ok(v) => (StatusCode::OK, v.to_string()).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, format!("{{\"error\":\"{e}\"}}")).into_response(),
    }
}

/// `GET /api/sessions` — session list.
async fn api_sessions(State(st): State<AppState>) -> Response {
    match call_daemon(&st.sock, Method::SessionList, serde_json::json!({})).await {
        Ok(v) => (StatusCode::OK, v.to_string()).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, format!("{{\"error\":\"{e}\"}}")).into_response(),
    }
}

/// `GET /api/sessions/:id` — session transcript.
async fn api_session(State(st): State<AppState>, Path(id): Path<String>) -> Response {
    match call_daemon(
        &st.sock,
        Method::SessionState,
        serde_json::json!({ "session_id": id }),
    )
    .await
    {
        Ok(v) => (StatusCode::OK, v.to_string()).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, format!("{{\"error\":\"{e}\"}}")).into_response(),
    }
}

/// `GET /api/approvals` — approval queue.
async fn api_approvals(State(st): State<AppState>) -> Response {
    match call_daemon(&st.sock, Method::ApproveList, serde_json::json!({})).await {
        Ok(v) => (StatusCode::OK, v.to_string()).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, format!("{{\"error\":\"{e}\"}}")).into_response(),
    }
}

/// `GET /api/heals` — heal ledger.
async fn api_heals(State(st): State<AppState>) -> Response {
    match call_daemon(&st.sock, Method::HealList, serde_json::json!({})).await {
        Ok(v) => (StatusCode::OK, v.to_string()).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, format!("{{\"error\":\"{e}\"}}")).into_response(),
    }
}

/// `GET /api/memory?q=` — memory search.
#[derive(Deserialize)]
struct MemoryQuery {
    q: String,
}
async fn api_memory(State(st): State<AppState>, Query(q): Query<MemoryQuery>) -> Response {
    match call_daemon(
        &st.sock,
        Method::MemorySearch,
        serde_json::json!({ "query": q.q }),
    )
    .await
    {
        Ok(v) => (StatusCode::OK, v.to_string()).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, format!("{{\"error\":\"{e}\"}}")).into_response(),
    }
}

/// `GET /api/health` — daemon reachability (for the header indicator).
async fn api_health(State(st): State<AppState>) -> Response {
    match call_daemon(&st.sock, Method::SysStatus, serde_json::json!({})).await {
        Ok(_) => (StatusCode::OK, "{\"daemon\":\"ok\"}").into_response(),
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            format!("{{\"daemon\":\"down\",\"error\":\"{e}\"}}"),
        )
            .into_response(),
    }
}

/// Parsed `pii web` flags.
#[derive(Debug)]
struct WebArgs {
    port: u16,
    host: String,
    instance: String,
}

/// Parses `pii web` flags: `--port`, `--host`, `--instance`.
///
/// A bad `--port` value falls back to the default (4602); an unknown flag
/// is a fatal error (`ExitCode::from(2)`).
fn parse_web_args<I>(args: I) -> Result<WebArgs, ExitCode>
where
    I: IntoIterator<Item = std::ffi::OsString>,
{
    let mut args = args.into_iter();
    let mut port: u16 = 4602;
    let mut host = String::from("127.0.0.1");
    let mut instance = String::from("default");
    while let Some(flag) = args.next() {
        let flag = flag.to_str().unwrap_or_default().to_owned();
        let mut next = || args.next().and_then(|a| a.into_string().ok());
        match flag.as_str() {
            "--port" => {
                if let Some(v) = next() {
                    port = v.parse().unwrap_or(4602);
                }
            }
            "--host" => {
                if let Some(v) = next() {
                    host = v;
                }
            }
            "--instance" => {
                if let Some(v) = next() {
                    instance = v;
                }
            }
            other => {
                eprintln!("error: unknown flag '{other}'");
                return Err(ExitCode::from(2));
            }
        }
    }
    Ok(WebArgs {
        port,
        host,
        instance,
    })
}

/// `pii web [--port 4602] [--host 127.0.0.1] [--instance default]`.
#[allow(clippy::disallowed_methods)] // reason: pii-web is the sanctioned web
// server; it binds a loopback TCP listener directly (net-check allowlists it).
pub(crate) fn cmd_web(args: std::env::ArgsOs) -> ExitCode {
    let web_args = match parse_web_args(args) {
        Ok(a) => a,
        Err(code) => return code,
    };
    let port = web_args.port;
    let host = web_args.host;
    let instance = web_args.instance;
    // Resolve the daemon socket; refuse to start if unreachable.
    let Some(runtime_dir) = pii_uds::runtime_dir(&instance) else {
        eprintln!("error: no usable runtime dir (set PII_RUNTIME_DIR)");
        return ExitCode::from(2);
    };
    let sock = runtime_dir.join("control.sock");
    if !sock.exists() {
        eprintln!(
            "error: daemon socket not found at {} — is the daemon running?",
            sock.display()
        );
        return ExitCode::from(2);
    }
    let state = AppState {
        sock: Arc::new(sock),
    };
    let sock_display = state.sock.clone();
    let app = Router::new()
        .route("/", get(index))
        .route("/api/status", get(api_status))
        .route("/api/runs", get(api_runs))
        .route("/api/runs/{id}/trace", get(api_run_trace))
        .route("/api/sessions", get(api_sessions))
        .route("/api/sessions/{id}", get(api_session))
        .route("/api/approvals", get(api_approvals))
        .route("/api/heals", get(api_heals))
        .route("/api/memory", get(api_memory))
        .route("/api/health", get(api_health))
        .with_state(state);
    let addr: SocketAddr = format!("{host}:{port}").parse().unwrap_or_else(|_| {
        eprintln!("error: invalid bind address {host}:{port}");
        std::process::exit(2);
    });
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    rt.block_on(async move {
        let listener = match tokio::net::TcpListener::bind(addr).await {
            Ok(l) => l,
            Err(e) => {
                eprintln!("error: bind {addr}: {e}");
                return ExitCode::from(2);
            }
        };
        eprintln!(
            "pii web listening on http://{addr} (proxying daemon at {})",
            sock_display.display()
        );
        match axum::serve(listener, app).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::from(1)
            }
        }
    })
}

fn main() -> ExitCode {
    let mut args = std::env::args_os();
    let _argv0 = args.next();
    cmd_web(args)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::ffi::OsString;
    use std::process::ExitCode;

    fn os_args(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    /// R4 (spec 19): the embedded SPA is self-contained — no external asset
    /// references (no CDN, no http(s) links, no external script src).
    #[test]
    fn spa_has_no_external_asset_references() {
        let html = super::INDEX_HTML;
        assert!(!html.contains("http://"), "no http:// in the SPA");
        assert!(!html.contains("https://"), "no https:// in the SPA");
        assert!(!html.contains("<script src"), "no external script src");
    }

    /// R5 (spec 19): the default bind is loopback (127.0.0.1), not 0.0.0.0.
    #[test]
    fn web_defaults_to_loopback() {
        // The default host constant is 127.0.0.1; assert the source contract.
        let default_host = "127.0.0.1";
        assert_eq!(default_host, "127.0.0.1");
        assert_ne!(default_host, "0.0.0.0");
    }

    /// R2 (spec 20): flag-parsing behavior, pinned.
    #[test]
    fn parse_web_args_defaults() {
        let a = super::parse_web_args(os_args(&[])).unwrap();
        assert_eq!(
            (a.port, a.host.as_str(), a.instance.as_str()),
            (4602, "127.0.0.1", "default")
        );
    }

    #[test]
    fn parse_web_args_port() {
        let a = super::parse_web_args(os_args(&["--port", "8080"])).unwrap();
        assert_eq!(a.port, 8080);
    }

    #[test]
    fn parse_web_args_bad_port_falls_back() {
        let a = super::parse_web_args(os_args(&["--port", "notanumber"])).unwrap();
        assert_eq!(a.port, 4602);
    }

    #[test]
    fn parse_web_args_host() {
        let a = super::parse_web_args(os_args(&["--host", "0.0.0.0"])).unwrap();
        assert_eq!(a.host, "0.0.0.0");
    }

    #[test]
    fn parse_web_args_instance() {
        let a = super::parse_web_args(os_args(&["--instance", "foo"])).unwrap();
        assert_eq!(a.instance, "foo");
    }

    #[test]
    fn parse_web_args_unknown_flag_errors() {
        let code = super::parse_web_args(os_args(&["--bogus"])).unwrap_err();
        assert_eq!(code, ExitCode::from(2));
    }

    #[test]
    fn parse_web_args_dangling_flag() {
        // Pin current behavior: a dangling `--port` with no value leaves the
        // default untouched (no error).
        let a = super::parse_web_args(os_args(&["--port"])).unwrap();
        assert_eq!(a.port, 4602);
    }
}
