//! Spec 108 R3: the subsystems' observability seams are attached in the
//! daemon's wiring, not only in their own unit tests. Each test builds the
//! engine/daemon the way `pii daemon` does, attaches ONE log observer, and
//! asserts the named event arrives through it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pii_observe::{Recorder, names};
use pii_server::daemon::Daemon;
use pii_server::daemon::config::Config;
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;

fn hello() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .join("fixtures/phase2/hello.json")
}

fn scratch(prefix: &str) -> tempfile::TempDir {
    pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), prefix).expect("tempdir")
}

/// An engine with the log attached BEFORE the daemon is built (as in
/// `pii daemon`), and a daemon built from `config_text`.
fn daemon_with_log(dir: &Path, config_text: &str) -> (Arc<Daemon>, Arc<Recorder>) {
    let workspace = dir.join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    let engine = Engine::new(EngineConfig {
        grant_ceiling: None,
        workspace,
        store_path: dir.join("store.redb"),
        model: format!("scripted:{}", hello().display()),
        sandbox: SandboxProfileName::Off,
        route_policy: std::collections::BTreeMap::new(),
        tiers: Vec::new(),
        session_runs_cap: pii_server::DEFAULT_SESSION_RUNS_CAP,
        llama_host: None,
        graph_dir: dir.join("graphs"),
        mqtt_tool_url: None,
    })
    .expect("engine");
    let log = Arc::new(Recorder::new());
    engine.attach_log(log.clone());
    let cfg_path = dir.join("pii.toml");
    std::fs::write(&cfg_path, config_text).unwrap();
    let cfg = Config::load(&cfg_path).expect("config");
    let d = Daemon::build(engine, &cfg, dir, cfg_path, Arc::new(pii_core::SystemClock))
        .expect("daemon");
    (d, log)
}

fn wait_for(log: &Recorder, kind: &str) -> bool {
    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        if !log.events_named(kind).is_empty() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

#[tokio::test(flavor = "multi_thread")]
async fn a_model_request_reaches_the_log() {
    let dir = scratch("logw-model");
    let (d, log) = daemon_with_log(dir.path(), "# none\n");
    let sid = d
        .engine
        .create_session(dir.path().display().to_string(), vec![], None);
    d.engine.prompt(&sid, &[], None).expect("prompt");
    assert!(
        wait_for(&log, names::MODEL_PROVIDER_IN_FORCE),
        "{:?}",
        log.events()
    );
    assert_ne!(
        log.events_named(names::MODEL_REQUEST_LATENCY),
        [] as [pii_observe::OwnedEvent; 0]
    );
}

#[test]
fn a_worker_state_change_reaches_the_log() {
    let dir = scratch("logw-worker");
    let cfg = "[[worker]]\nname = \"w\"\nprogram = \"/nonexistent/pii-worker-prog\"\n";
    let (_d, log) = daemon_with_log(dir.path(), cfg);
    let ev = log.events_named(names::WORKER_STATE_CHANGED);
    assert!(!ev.is_empty(), "{:?}", log.events());
    assert!(
        ev.iter()
            .all(|e| e.fields.get("worker") == Some(&serde_json::json!("w"))),
        "{ev:?}"
    );
}

#[test]
fn a_failing_channel_poll_reaches_the_log() {
    let dir = scratch("logw-channel");
    let inbox = dir.path().join("in.jsonl");
    std::fs::write(&inbox, "").unwrap();
    let cfg = format!(
        "[[channel]]\nname = \"c\"\nkind = \"cli\"\ngraph = \"g\"\ninbox = \"{}\"\noutbox = \"{}\"\n",
        inbox.display(),
        dir.path().join("out.jsonl").display()
    );
    let (d, log) = daemon_with_log(dir.path(), &cfg);
    let channel = Arc::clone(&d.ctx.channels.lock().get("c").unwrap().channel);
    // No fs capability on this ctx: the poll fails and must be reported.
    let ctx = pii_core::Ctx::root(pii_core::CapSet::empty(), pii_core::Budget::unlimited());
    assert!(channel.poll(&ctx, None).is_err());
    assert_eq!(
        log.events_named(names::CHANNEL_POLL_FAILED).len(),
        1,
        "{:?}",
        log.events()
    );
}

#[test]
fn a_denied_egress_reaches_the_log() {
    let dir = scratch("logw-net");
    let (d, log) = daemon_with_log(dir.path(), "# no grants\n");
    let ctx = pii_core::Ctx::root(pii_core::CapSet::empty(), pii_core::Budget::unlimited());
    let target = pii_net::authorize::Target {
        host: "example.invalid".into(),
        port: 443,
    };
    let r = d.ctx.net.connect(
        &ctx,
        &target,
        "http:example.invalid",
        100,
        pii_net::Scheme::Http,
    );
    assert!(r.is_err());
    assert!(
        !log.events_named(names::NET_EGRESS_DENIED).is_empty()
            || !log.events_named(names::NET_DIAL_FAILED).is_empty(),
        "{:?}",
        log.events()
    );
}

#[test]
fn a_store_write_failure_reaches_the_log() {
    let dir = scratch("logw-store");
    let (d, log) = daemon_with_log(dir.path(), "# none\n");
    let sid = d
        .engine
        .create_session(dir.path().display().to_string(), vec![], None);
    d.engine.store.set_fail_session_writes_for_test(true);
    d.engine.close_session(&sid).ok();
    d.engine.store.set_fail_session_writes_for_test(false);
    assert!(
        !log.events_named(names::STORE_PERSIST_ERROR).is_empty(),
        "{:?}",
        log.events()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_memory_search_rpc_reaches_the_log() {
    let dir = scratch("logw-memory");
    let (d, log) = daemon_with_log(dir.path(), "# none\n");
    let stream = pii_server::engine::loopback::spawn_loopback(Arc::clone(&d.engine));
    let (r, w) = tokio::io::split(stream);
    let info = pii_proto::sys::PeerInfo {
        name: "logw".into(),
        version: "0".into(),
        build: "test".into(),
    };
    let (client, _) = pii_proto::client::Client::connect(r, w, info)
        .await
        .expect("connect");
    let _ = client
        .call(
            pii_proto::method::Method::MemorySearch,
            serde_json::json!({"q": "anything"}),
        )
        .await;
    client.shutdown();
    assert_eq!(
        log.events_named(names::MEMORY_SEARCH).len(),
        1,
        "{:?}",
        log.events()
    );
}

const SUDO: &str = r#"name = "sudo"
entry = "b"
caps = ["fs_read", "fs_write", "proc"]

[[node]]
id = "b"
kind = "tool"
tool = "bash"
args = { command = "sudo ls" }
"#;

/// A bash deny-list hit inside a REAL run reaches the log as `tools.*`
/// events (spec 108 R4; before it only `core.cap.denied` was emitted).
#[tokio::test(flavor = "multi_thread")]
async fn a_deny_list_hit_in_a_run_reaches_the_log() {
    let dir = scratch("logw-tools");
    std::fs::create_dir_all(dir.path().join("graphs")).unwrap();
    std::fs::write(dir.path().join("graphs/sudo.toml"), SUDO).unwrap();
    let (d, log) = daemon_with_log(dir.path(), "# none\n");
    d.engine.start_run("sudo", &serde_json::json!({})).unwrap();
    assert!(
        wait_for(&log, names::TOOLS_DENY_LIST_HIT),
        "{:?}",
        log.events()
    );
    let calls = log.events_named(names::TOOLS_CALL);
    assert_eq!(calls.len(), 1, "{:?}", log.events());
    assert_eq!(
        calls[0].fields.get("outcome"),
        Some(&serde_json::json!("denied"))
    );
}

/// Spec 116 R4: with the observer fan-out detached the same run still reaches
/// its terminal state, but no `tools.*` event reaches the log. The
/// attached-case test above is the mutation control: if `detach_observers`
/// did nothing, `tools.call` would arrive here and this test would fail.
#[tokio::test(flavor = "multi_thread")]
async fn a_detached_observer_fan_out_logs_nothing_and_the_run_still_finishes() {
    let dir = scratch("logw-detached");
    std::fs::create_dir_all(dir.path().join("graphs")).unwrap();
    std::fs::write(dir.path().join("graphs/sudo.toml"), SUDO).unwrap();
    let (d, log) = daemon_with_log(dir.path(), "# none\n");
    assert!(!d.engine.observers_detached(), "attached is the default");
    d.engine.detach_observers();
    let run = d.engine.start_run("sudo", &serde_json::json!({})).unwrap();
    let end = Instant::now() + Duration::from_secs(10);
    let finished = loop {
        let h = d.engine.store.get_run(&run).unwrap();
        if h.is_some_and(|h| h.status != pii_abi::RunStatus::Running) {
            break true;
        }
        if Instant::now() > end {
            break false;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(finished, "the run must still reach a terminal state");
    assert!(
        log.events_named(names::TOOLS_CALL).is_empty(),
        "{:?}",
        log.events()
    );
}

/// Spec 177: a cron entry that cannot start is an event, not a silence.
#[test]
fn a_cron_entry_that_cannot_start_is_reported() {
    let dir = scratch("logw-cron-refused");
    std::fs::create_dir_all(dir.path().join("graphs")).unwrap();
    let cfg = "[[cron]]\nname = \"nightly\"\ngraph = \"no-such-graph\"\n";
    let (d, log) = daemon_with_log(dir.path(), cfg);
    d.start_cron_entries();
    let ev = log.events_named(names::CRON_START_REFUSED);
    assert_eq!(ev.len(), 1, "{:?}", log.events());
    assert!(d.ctx.cron_runs.lock().is_empty());
}

/// Spec 193: a started cron entry is one `cron.fired` naming the entry and the run, and no refusal.
#[tokio::test(flavor = "multi_thread")]
async fn a_started_cron_entry_is_one_firing() {
    let dir = scratch("logw-cron-fired");
    std::fs::create_dir_all(dir.path().join("graphs")).unwrap();
    let tick = "name = \"tick\"\nentry = \"t\"\n\n[[node]]\nid = \"t\"\nkind = \"timer\"\nin_ms = 3600000\n";
    std::fs::write(dir.path().join("graphs/tick.toml"), tick).unwrap();
    let cfg = "[[cron]]\nname = \"nightly\"\ngraph = \"tick\"\n";
    let (d, log) = daemon_with_log(dir.path(), cfg);
    d.start_cron_entries();
    let ev = log.events_named(names::CRON_FIRED);
    assert_eq!(ev.len(), 1, "{:?}", log.events());
    assert_eq!(
        ev[0].fields.get("cron"),
        Some(&serde_json::json!("nightly"))
    );
    let run = d
        .ctx
        .cron_runs
        .lock()
        .keys()
        .next()
        .cloned()
        .expect("a tracked run");
    assert_eq!(ev[0].fields.get("run"), Some(&serde_json::json!(run)));
    assert_eq!(
        log.events_named(names::CRON_START_REFUSED),
        [] as [pii_observe::OwnedEvent; 0]
    );
}
