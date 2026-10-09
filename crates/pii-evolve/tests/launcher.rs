//! R2′/R4 (spec 27): `pii-evolve`'s driver is a thin UDS launcher — it sends
//! `run.start` for `starter/boss-watch` to a daemon and records the launched
//! run id + cycle in `state.md`. These tests drive it against a stub UDS
//! server (no real daemon, no subprocess).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use pii_evolve::driver::{BossLaunch, LaunchedRun, run_cycle};
use pii_evolve::state::StateDoc;
use pii_proto::sys::{HelloResult, Limits, PeerInfo};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;

/// A shared slot for the stub to record what it received.
type Received<T> = Arc<Mutex<Option<T>>>;

/// A stub daemon: accepts one UDS connection, answers `sys.hello`, then
/// answers `run.start` with a fixed run id. Records the received method and
/// params for assertion.
struct StubDaemon {
    run_id: String,
    received_method: Received<String>,
    received_params: Received<serde_json::Value>,
}

impl StubDaemon {
    fn new(run_id: &str) -> (Self, Received<String>, Received<serde_json::Value>) {
        let received_method = Arc::new(Mutex::new(None));
        let received_params = Arc::new(Mutex::new(None));
        (
            Self {
                run_id: run_id.to_owned(),
                received_method: Arc::clone(&received_method),
                received_params: Arc::clone(&received_params),
            },
            received_method,
            received_params,
        )
    }

    async fn serve(self, listener: UnixListener) {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let (r, mut w) = stream.split();
        let mut lines = BufReader::new(r).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if line.trim().is_empty() {
                continue;
            }
            let req: serde_json::Value = serde_json::from_str(&line).expect("parse request");
            let method = req
                .get("method")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_owned();
            let id = req.get("id").cloned().unwrap_or(serde_json::Value::Null);
            let params = req
                .get("params")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let resp = match method.as_str() {
                "sys.hello" => {
                    let hello = HelloResult {
                        protocol: String::from("pii.v2"),
                        server: PeerInfo {
                            name: String::from("stub"),
                            version: String::from("0"),
                            build: String::from("test"),
                        },
                        encoding: String::from("json"),
                        capabilities: Vec::new(),
                        limits: Limits {
                            max_frame_bytes: 1 << 20,
                            max_subscriptions: 4,
                        },
                    };
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": serde_json::to_value(&hello).unwrap(),
                    })
                }
                "run.start" => {
                    *self.received_method.lock() = Some(method.clone());
                    *self.received_params.lock() = Some(params);
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": { "run_id": self.run_id },
                    })
                }
                other => {
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32601, "message": format!("method not found: {other}") },
                    })
                }
            };
            let mut line = serde_json::to_string(&resp).expect("serialize response");
            line.push('\n');
            w.write_all(line.as_bytes()).await.expect("write response");
            w.flush().await.expect("flush");
        }
    }
}

/// Binds a temp UDS socket (std listener). Returns `(path, std_listener,
/// _keepalive)` — the `TempDir` must stay alive for the socket file to exist.
/// The caller converts to a tokio listener inside its own runtime thread.
fn bind_socket(tag: &str) -> (PathBuf, std::os::unix::net::UnixListener, tempfile::TempDir) {
    let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp"));
    std::fs::create_dir_all(&root).expect("create crate-local _tmp");
    let dir = tempfile::Builder::new()
        .prefix(tag)
        .tempdir_in(&root)
        .expect("tempdir");
    let path = dir.path().join("control.sock");
    let listener = std::os::unix::net::UnixListener::bind(&path).expect("bind");
    (path, listener, dir)
}

/// Runs the stub server on a dedicated thread + current-thread runtime.
fn spawn_stub(stub: StubDaemon, listener: std::os::unix::net::UnixListener) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("stub runtime");
        rt.block_on(async move {
            listener.set_nonblocking(true).expect("nonblocking");
            let listener = UnixListener::from_std(listener).expect("tokio listener");
            stub.serve(listener).await;
        });
    });
}

/// R2′: after `run_cycle`, `state.md` carries `last_run_id` equal to the
/// stub's run id, `cycle_count` incremented, and `phase = "watching"`.
#[test]
fn launch_records_run_id_in_state_md() {
    let (path, listener, _keepalive) = bind_socket("launch-state");
    let (stub, _m, _p) = StubDaemon::new("r-boss-1");
    // The stub server runs on its own thread + runtime; `run_cycle` builds
    // its own current-thread runtime, so it must run on the plain test thread.
    spawn_stub(stub, listener);

    let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp"));
    std::fs::create_dir_all(&root).expect("create crate-local _tmp");
    let state_dir = tempfile::Builder::new()
        .prefix("launch-state-dir")
        .tempdir_in(&root)
        .expect("state dir");
    let cfg = BossLaunch::new(path, state_dir.path().to_path_buf());
    let launched: LaunchedRun = run_cycle(&cfg).expect("run_cycle");

    assert_eq!(launched.run_id, "r-boss-1");
    assert_eq!(launched.cycle, 1, "first cycle is 1");

    let state = StateDoc::load(state_dir.path()).expect("load state");
    assert_eq!(
        state.properties.get("last_run_id").map(String::as_str),
        Some("r-boss-1"),
        "state.md must record the launched run id"
    );
    assert_eq!(
        state.properties.get("cycle_count").map(String::as_str),
        Some("1"),
        "state.md must record cycle_count"
    );
    assert_eq!(state.phase, "watching", "phase must be 'watching'");
}

/// R4: the driver sends `run.start` for `starter/boss-watch` with
/// `input.cycle == cycle_count + 1`, and spawns no subprocess.
#[test]
fn run_cycle_sends_run_start() {
    let (path, listener, _keepalive) = bind_socket("launch-rpc");
    let (stub, method, params) = StubDaemon::new("r-boss-2");
    spawn_stub(stub, listener);

    let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp"));
    std::fs::create_dir_all(&root).expect("create crate-local _tmp");
    let state_dir = tempfile::Builder::new()
        .prefix("launch-rpc-dir")
        .tempdir_in(&root)
        .expect("state dir");
    // Pre-seed cycle_count = 4 so the next cycle is 5.
    let mut state = StateDoc::default();
    state
        .properties
        .insert(String::from("cycle_count"), String::from("4"));
    state.save(state_dir.path()).expect("seed state");

    let cfg = BossLaunch::new(path, state_dir.path().to_path_buf());
    let launched = run_cycle(&cfg).expect("run_cycle");
    assert_eq!(launched.cycle, 5, "cycle must be cycle_count + 1");

    // Give the stub a moment to record the request.
    std::thread::sleep(std::time::Duration::from_millis(200));
    let m = method.lock().clone();
    assert_eq!(m.as_deref(), Some("run.start"), "must send run.start");
    let p = params.lock().clone().expect("params");
    assert_eq!(
        p.get("graph").and_then(serde_json::Value::as_str),
        Some("starter/boss-watch"),
        "must launch the boss-watch graph"
    );
    assert_eq!(
        p.get("input")
            .and_then(|i| i.get("cycle"))
            .and_then(serde_json::Value::as_u64),
        Some(5),
        "input.cycle must be cycle_count + 1"
    );
    assert_eq!(
        p.get("input")
            .and_then(|i| i.get("state_dir"))
            .and_then(serde_json::Value::as_str),
        Some(state_dir.path().to_str().unwrap()),
        "input.state_dir must be the state dir"
    );
}

/// R4 (no subprocess): the driver source must not shell out to `cargo xtask`.
#[test]
fn driver_spawns_no_subprocess() {
    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/driver.rs"),
    )
    .expect("read driver.rs");
    assert!(
        !src.contains("cargo xtask") && !src.contains("Command::new"),
        "driver must not spawn a subprocess or shell out to cargo xtask"
    );
}
