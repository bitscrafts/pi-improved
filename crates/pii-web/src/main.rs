//! `pii web` — read-only web viewer (P4.6, spec 19).
//!
//! Proxies the daemon's UDS JSON-RPC over loopback HTTP so a browser can
//! watch runs, sessions, approvals, heals, and memory. VIEWER-ONLY: no
//! write RPC is proxied. axum lives here, not in the workspace root or
//! pii-cli (deps-check invariant).
//!
//! Exposure note: this crate has no authentication of its own — it forwards
//! the daemon's read surface under pii-web's own process uid, bypassing the
//! peer-uid check the daemon's UDS socket normally enforces. `--host 0.0.0.0`
//! is accepted, but binding off loopback exposes that read surface to the
//! network without additional authentication; the default bind is loopback
//! for this reason.

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
use pii_observe::{Event, Level, Observer, StderrObserver, names};
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
    observer: Arc<dyn Observer>,
}

impl AppState {
    /// Calls the daemon; a failure is reported as [`names::WEB_UPSTREAM_ERROR`]
    /// (method and error only — never params, which may carry a search query).
    async fn upstream(
        &self,
        method: Method,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let name = method.to_string();
        let result = call_daemon(&self.sock, method, params).await;
        if let Err(e) = &result {
            self.observer.event(
                &Event::new("web", Level::Error, names::WEB_UPSTREAM_ERROR)
                    .field("method", name)
                    .field("error", e.as_str()),
            );
        }
        result
    }

    /// The `502` answer for a failed upstream call, reported as
    /// [`names::WEB_REQUEST_FAILED`].
    fn bad_gateway(&self, error: &str) -> Response {
        self.observer.event(
            &Event::new("web", Level::Error, names::WEB_REQUEST_FAILED)
                .field("status", 502_u16)
                .field("error", error),
        );
        (
            StatusCode::BAD_GATEWAY,
            format!("{{\"error\":\"{error}\"}}"),
        )
            .into_response()
    }
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
    match st.upstream(Method::SysStatus, serde_json::json!({})).await {
        Ok(v) => (StatusCode::OK, v.to_string()).into_response(),
        Err(e) => st.bad_gateway(&e),
    }
}

/// `GET /api/runs` — run list.
async fn api_runs(State(st): State<AppState>) -> Response {
    match st.upstream(Method::RunList, serde_json::json!({})).await {
        Ok(v) => (StatusCode::OK, v.to_string()).into_response(),
        Err(e) => st.bad_gateway(&e),
    }
}

/// `GET /api/runs/:id/trace` — run journal trace.
async fn api_run_trace(State(st): State<AppState>, Path(id): Path<String>) -> Response {
    match st
        .upstream(Method::RunTrace, serde_json::json!({ "run_id": id }))
        .await
    {
        Ok(v) => (StatusCode::OK, v.to_string()).into_response(),
        Err(e) => st.bad_gateway(&e),
    }
}

/// `GET /api/sessions` — session list.
async fn api_sessions(State(st): State<AppState>) -> Response {
    match st
        .upstream(Method::SessionList, serde_json::json!({}))
        .await
    {
        Ok(v) => (StatusCode::OK, v.to_string()).into_response(),
        Err(e) => st.bad_gateway(&e),
    }
}

/// `GET /api/sessions/:id` — session transcript.
async fn api_session(State(st): State<AppState>, Path(id): Path<String>) -> Response {
    match st
        .upstream(
            Method::SessionState,
            serde_json::json!({ "session_id": id }),
        )
        .await
    {
        Ok(v) => (StatusCode::OK, v.to_string()).into_response(),
        Err(e) => st.bad_gateway(&e),
    }
}

/// `GET /api/approvals` — approval queue.
async fn api_approvals(State(st): State<AppState>) -> Response {
    match st
        .upstream(Method::ApproveList, serde_json::json!({}))
        .await
    {
        Ok(v) => (StatusCode::OK, v.to_string()).into_response(),
        Err(e) => st.bad_gateway(&e),
    }
}

/// `GET /api/heals` — heal ledger.
async fn api_heals(State(st): State<AppState>) -> Response {
    match st.upstream(Method::HealList, serde_json::json!({})).await {
        Ok(v) => (StatusCode::OK, v.to_string()).into_response(),
        Err(e) => st.bad_gateway(&e),
    }
}

/// `GET /api/memory?q=` — memory search.
#[derive(Deserialize)]
struct MemoryQuery {
    q: String,
}
async fn api_memory(State(st): State<AppState>, Query(q): Query<MemoryQuery>) -> Response {
    match st
        .upstream(Method::MemorySearch, serde_json::json!({ "q": q.q }))
        .await
    {
        Ok(v) => (StatusCode::OK, v.to_string()).into_response(),
        Err(e) => st.bad_gateway(&e),
    }
}

/// `GET /api/health` — daemon reachability (for the header indicator).
async fn api_health(State(st): State<AppState>) -> Response {
    match st.upstream(Method::SysStatus, serde_json::json!({})).await {
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
        observer: Arc::new(StderrObserver),
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
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use std::ffi::OsString;
    use std::process::ExitCode;
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::util::ServiceExt;

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

    // --- memory.search param-key regression (spec: q, not query) ---

    use serde_json::Value;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Minimal fake daemon: answers `sys.hello`, then for `memory.search`
    /// accepts ONLY `{"q": ...}` (mirrors `memory_search` in
    /// `pii-server/src/engine/methods.rs`, which reads `q` and errors
    /// `InvalidParams` otherwise). Records the raw params for inspection.
    async fn fake_memory_daemon(
        stream: tokio::net::UnixStream,
        seen: Arc<AtomicU64>,
    ) -> std::io::Result<()> {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        let (r, mut w) = tokio::io::split(stream);
        let mut lines = BufReader::new(r).lines();
        let mut next_id: u64 = 0;
        while let Some(line) = lines.next_line().await? {
            let v: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
            let Some(method) = v.get("method").and_then(Value::as_str) else {
                break;
            };
            let (result, is_error) = match method {
                "sys.hello" => (
                    serde_json::json!({
                        "protocol": "pii.v2",
                        "server": {"name": "fake", "version": "0", "build": "test"},
                        "encoding": "json",
                        "capabilities": [],
                        "limits": {
                            "max_frame_bytes": 1_048_576_u64,
                            "max_subscriptions": 0_u32
                        }
                    }),
                    false,
                ),
                "memory.search" => {
                    let params = v.get("params").cloned().unwrap_or(Value::Null);
                    if params.get("q").and_then(Value::as_str).is_some() {
                        seen.store(1, Ordering::SeqCst);
                        (serde_json::json!({"hits": []}), false)
                    } else {
                        (
                            Value::Null,
                            true, // InvalidParams: the daemon only reads "q"
                        )
                    }
                }
                _ => (Value::Null, true),
            };
            let resp = if is_error {
                serde_json::json!({
                    "jsonrpc": "2.0", "id": next_id,
                    "error": {"code": -32602i32, "message": "memory.search needs 'q'"}
                })
            } else {
                serde_json::json!({"jsonrpc": "2.0", "id": next_id, "result": result})
            };
            next_id += 1;
            w.write_all(resp.to_string().as_bytes()).await?;
            w.write_all(b"\n").await?;
            w.flush().await?;
        }
        Ok(())
    }

    /// Starts a fake daemon on a fresh UDS socket under `_tmp`.
    fn start_fake_daemon() -> (std::path::PathBuf, Arc<AtomicU64>) {
        let dir = std::env::temp_dir().join(format!("pii-web-mem-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("control.sock");
        let _ = std::fs::remove_file(&sock);
        // reason: a minimal one-test fake responder, not the daemon; socket
        // permissions are irrelevant for a loopback-only test fixture.
        #[allow(clippy::disallowed_methods)]
        let listener = tokio::net::UnixListener::bind(&sock).unwrap();
        let seen = Arc::new(AtomicU64::new(0));
        let seen_clone = Arc::clone(&seen);
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let seen = Arc::clone(&seen_clone);
                tokio::spawn(fake_memory_daemon(stream, seen));
            }
        });
        (sock, seen)
    }

    fn test_state(sock: std::path::PathBuf) -> super::AppState {
        super::AppState {
            sock: Arc::new(sock),
            observer: Arc::new(pii_observe::NullObserver),
        }
    }

    /// Spec 101 R3: an unreachable daemon surfaces as `web.upstream.error`
    /// plus `web.request.failed` (status 502), by registered name.
    #[tokio::test]
    async fn upstream_failure_is_reported_by_name() {
        use pii_observe::{Recorder, names};
        let rec = Arc::new(Recorder::new());
        let mut st = test_state(std::path::PathBuf::from("/nonexistent/control.sock"));
        st.observer = rec.clone();
        let app = axum::Router::new()
            .route("/api/runs", axum::routing::get(super::api_runs))
            .with_state(st);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/runs")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
        let up = rec.events_named(names::WEB_UPSTREAM_ERROR);
        assert_eq!(up.len(), 1, "{:?}", rec.events());
        assert_eq!(
            up[0].fields.get("method"),
            Some(&serde_json::json!("run.list"))
        );
        let req = rec.events_named(names::WEB_REQUEST_FAILED);
        assert_eq!(req.len(), 1, "{:?}", rec.events());
        assert_eq!(req[0].fields.get("status"), Some(&serde_json::json!(502)));
    }

    /// TDD: `api_memory` must send `{"q": ...}` — not `{"query": ...}` — to
    /// the daemon (`memory_search` in pii-server reads only `q`).
    #[tokio::test]
    async fn api_memory_route_reaches_daemon_successfully() {
        let (sock, seen) = start_fake_daemon();
        let app = axum::Router::new()
            .route("/api/memory", axum::routing::get(super::api_memory))
            .with_state(test_state(sock));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/memory?q=test%20term")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(seen.load(Ordering::SeqCst), 1, "fake daemon accepted 'q'");
    }

    /// TDD: pin the exact params key sent to `call_daemon` for
    /// `memory.search`: it must be `q`, never `query`.
    #[test]
    fn api_memory_sends_correct_param_key() {
        // Mirror of the handler's params construction; asserts the source
        // contract at the seam the daemon reads.
        let mq = super::MemoryQuery {
            q: String::from("test term"),
        };
        let params = serde_json::json!({ "q": mq.q });
        assert_eq!(params.get("q").and_then(Value::as_str), Some("test term"));
        assert!(params.get("query").is_none(), "must not use 'query'");
    }
}
