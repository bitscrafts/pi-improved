//! Spec 80: heavy RPC methods must not block the async worker. A heartbeat
//! task ticks every 10 ms on a `current_thread` runtime while the method runs
//! against a deliberately slow stub handler; no tick may be late by > 50 ms.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::print_stderr
)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use pii_proto::method::Method;
use pii_proto::{Request, Response};

use crate::engine::conn::{PeerIdentity, SubscriptionRegistry, serve_connection_with};
use crate::{Engine, EngineConfig};

const TICK: Duration = Duration::from_millis(10);
const MAX_LATE: Duration = Duration::from_millis(50);
const SLOW: Duration = Duration::from_millis(300);

/// Stub handler: slow (blocking sleep) for every method except `sys.version`.
fn slow_stub(
    _engine: &Arc<Engine>,
    req: &Request,
    _peer: PeerIdentity,
    _wtx: &mpsc::UnboundedSender<String>,
    _subs: &SubscriptionRegistry,
) -> Response {
    if req.method != Method::SysVersion {
        std::thread::sleep(SLOW);
    }
    Response::ok(req.id.clone(), serde_json::json!({"stub": true}))
}

fn engine_in(dir: &std::path::Path) -> Arc<Engine> {
    Engine::new(EngineConfig {
        grant_ceiling: None,
        workspace: dir.to_path_buf(),
        store_path: dir.join("store.redb"),
        model: format!(
            "scripted:{}/../../fixtures/phase2/hello.json",
            env!("CARGO_MANIFEST_DIR")
        ),
        sandbox: pii_tools::sandbox::SandboxProfileName::Off,
        route_policy: std::collections::BTreeMap::new(),
        tiers: Vec::new(),
        session_runs_cap: crate::DEFAULT_SESSION_RUNS_CAP,
        llama_host: None,
        graph_dir: dir.join("graphs"),
        mqtt_tool_url: None,
    })
    .expect("engine")
}

async fn send_line<W: tokio::io::AsyncWrite + Unpin>(
    w: &mut W,
    method: &str,
    id: u32,
    params: &str,
) {
    let line = format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"{method}","params":{params}}}"#);
    w.write_all(line.as_bytes()).await.expect("write");
    w.write_all(b"\n").await.expect("nl");
    w.flush().await.expect("flush");
}

/// Returns the worst observed tick lateness while `method` is served.
async fn max_tick_delay(method: &str) -> Duration {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("_tmp");
    std::fs::create_dir_all(&root).expect("create crate-local _tmp");
    let dir = tempfile::Builder::new()
        .prefix("heartbeat-")
        .tempdir_in(root)
        .expect("tempdir");
    let engine = engine_in(dir.path());
    let (client, server) = tokio::io::duplex(64 * 1024);
    let serve = tokio::spawn(serve_connection_with(
        engine,
        server,
        PeerIdentity::InProcess,
        slow_stub,
    ));
    let (r, mut w) = tokio::io::split(client);
    let mut lines = BufReader::new(r).lines();
    send_line(
        &mut w,
        "sys.hello",
        1,
        r#"{"protocol":"pii.v2","client":{"name":"t","version":"0","build":"t"},"encodings":["json"],"features":[]}"#,
    )
    .await;
    lines.next_line().await.expect("hello").expect("hello line");

    let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel::<()>();
    let beat = tokio::spawn(async move {
        let mut worst = Duration::ZERO;
        let mut last = Instant::now();
        loop {
            tokio::select! {
                _ = &mut stop_rx => {
                    // Count the tick that was pending when we were stopped.
                    worst = worst.max(last.elapsed().saturating_sub(TICK));
                    break;
                }
                () = tokio::time::sleep(TICK) => {
                    let now = Instant::now();
                    worst = worst.max(now.duration_since(last).saturating_sub(TICK));
                    last = now;
                }
            }
        }
        worst
    });
    tokio::time::sleep(3 * TICK).await;
    send_line(&mut w, method, 2, "{}").await;
    let line = lines.next_line().await.expect("resp").expect("resp line");
    assert!(line.contains("\"stub\""), "stub answered: {line}");
    let _ = stop_tx.send(());
    let worst = beat.await.expect("beat");
    drop(w);
    drop(lines);
    let _ = serve.await;
    worst
}

async fn assert_not_blocked(method: &str) {
    let worst = max_tick_delay(method).await;
    eprintln!("{method}: worst tick delay = {worst:?}");
    assert!(
        worst <= MAX_LATE,
        "{method} blocked the async worker: tick late by {worst:?}"
    );
}

#[tokio::test]
async fn heartbeat_control_light_method_never_late() {
    assert_not_blocked("sys.version").await;
}

#[tokio::test]
async fn heartbeat_memory_search() {
    assert_not_blocked("memory.search").await;
}

#[tokio::test]
async fn heartbeat_memory_put() {
    assert_not_blocked("memory.put").await;
}

#[tokio::test]
async fn heartbeat_memory_consolidate() {
    assert_not_blocked("memory.consolidate").await;
}

#[tokio::test]
async fn heartbeat_worker_reload() {
    assert_not_blocked("worker.reload").await;
}

#[tokio::test]
async fn heartbeat_run_trace() {
    assert_not_blocked("run.trace").await;
}

#[tokio::test]
async fn heartbeat_session_prompt() {
    assert_not_blocked("session.prompt").await;
}

/// Spec 95: `event.subscribe` replays the journal inline (measured 102 ms for
/// a 5000-event journal, above the 50 ms bar), so it runs on the blocking pool.
#[tokio::test]
async fn heartbeat_event_subscribe() {
    assert_not_blocked("event.subscribe").await;
}

/// Spec 95 R4: this test's own light-method control (same harness, a method
/// the stub answers instantly).
#[tokio::test]
async fn heartbeat_control_light_method_for_event_subscribe() {
    assert_not_blocked("sys.version").await;
}
