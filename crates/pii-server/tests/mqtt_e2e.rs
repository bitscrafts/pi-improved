//! Spec 117: MQTT end to end through a real daemon and the in-process loopback
//! broker — a device message starts a run, a publish node answers on the
//! broker, a dead broker is `Degraded` in health, counters reach `sys.status`,
//! and a credential never reaches a status payload or an event.

#![allow(
    clippy::print_stdout,
    clippy::disallowed_macros,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

#[path = "../../pii-channels/tests/mqtt_broker/mod.rs"]
mod mqtt_broker;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use mqtt_broker::Broker;
use pii_proto::method::Method;
use pii_proto::sys::PeerInfo;
use pii_server::daemon::Daemon;
use pii_server::daemon::config::Config;
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;

const CANARY: &str = "CANARY-117-e2e-secret";

/// `show` lifts the message payload; `pub` republishes it (default payload =
/// the predecessor's output) with `QoS` and retain stated.
const GRAPH: &str = r#"name = "mq"
entry = "show"
caps = ["net"]

[[node]]
id = "show"
kind = "transform"
expr = "$.input.payload"

[[node]]
id = "pub"
kind = "mqtt"
broker = "plant"
op = "publish"
topic = "out/echo"
qos = 1
retain = true

[[edge]]
from = "show"
port = "ok"
to = "pub"
"#;

fn hello() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/phase2/hello.json")
}

fn scratch(p: &str) -> tempfile::TempDir {
    pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), p).expect("tempdir")
}

fn daemon_in(dir: &Path, config_text: &str) -> Result<Arc<Daemon>, String> {
    daemon_with(dir, config_text, |_| {})
}

fn daemon_with(
    dir: &Path,
    config_text: &str,
    setup: impl FnOnce(&Arc<Engine>),
) -> Result<Arc<Daemon>, String> {
    std::fs::create_dir_all(dir.join("ws")).unwrap();
    std::fs::create_dir_all(dir.join("graphs")).unwrap();
    std::fs::write(dir.join("graphs/mq.toml"), GRAPH).unwrap();
    let engine = Engine::new(EngineConfig {
        grant_ceiling: None,
        workspace: dir.join("ws"),
        store_path: dir.join("store.redb"),
        model: format!("scripted:{}", hello().display()),
        sandbox: SandboxProfileName::Off,
        route_policy: std::collections::BTreeMap::new(),
        tiers: Vec::new(),
        session_runs_cap: pii_server::DEFAULT_SESSION_RUNS_CAP,
        llama_host: None,
        models: pii_server::daemon::empty_model_roles(),
        graph_dir: dir.join("graphs"),
        mqtt_tool_url: None,
    })
    .expect("engine");
    let cfg_path = dir.join("pii.toml");
    std::fs::write(&cfg_path, config_text).unwrap();
    let mut cfg = Config::load(&cfg_path).map_err(|e| e.to_string())?;
    cfg.core.durable_dir = Some(dir.join("durable").display().to_string());
    setup(&engine);
    Daemon::build(engine, &cfg, dir, cfg_path, Arc::new(pii_core::SystemClock))
        .map_err(|e| e.to_string())
}

async fn call(engine: &Arc<Engine>, m: Method, params: serde_json::Value) -> serde_json::Value {
    let stream = pii_server::engine::loopback::spawn_loopback(Arc::clone(engine));
    let (r, w) = tokio::io::split(stream);
    let info = PeerInfo {
        name: "spec-117".into(),
        version: "0".into(),
        build: "test".into(),
    };
    let (client, _) = pii_proto::client::Client::connect(r, w, info)
        .await
        .expect("connect");
    let v = client.call(m, params).await.expect("call");
    client.shutdown();
    v
}

fn cfg_for(port: u16, extra: &str) -> String {
    format!(
        "[[mqtt]]\nname = \"plant\"\nurl = \"mqtt://127.0.0.1:{port}\"\n\
         topics = [\"sensors/#\"]\nqos = 1\ngraph = \"mq\"\npoll_interval_ms = 20\n\
         reconnect_min_ms = 20\nreconnect_max_ms = 80\n{extra}"
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn a_device_message_runs_the_graph_and_the_publish_node_answers_the_broker() {
    let b = Broker::start(false);
    let dir = scratch("mqtt-e2e-");
    let d = daemon_in(dir.path(), &cfg_for(b.port, "")).unwrap();
    let _h = d.spawn_background();
    for _ in 0..200 {
        if !b.subscribed().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    b.push("sensors/temp", br#"{"t":21}"#);
    let mut seen = Vec::new();
    for _ in 0..200 {
        seen = b.published();
        if !seen.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(seen.len(), 1, "exactly one publish reached the broker");
    println!("BROKER VIEW: {seen:?}");
    assert_eq!(seen[0].topic, "out/echo");
    assert_eq!(seen[0].qos, 1);
    assert!(seen[0].retain);
    let echoed: serde_json::Value = serde_json::from_slice(&seen[0].payload).unwrap();
    assert_eq!(
        echoed,
        serde_json::json!({"t": 21}),
        "JSON parsed, not text"
    );

    // The publish happens mid-run: wait for the run to land in the store.
    let mut runs = serde_json::Value::Null;
    for _ in 0..200 {
        runs = call(&d.engine, Method::RunList, serde_json::json!({})).await;
        if runs
            .as_array()
            .is_some_and(|a| a.iter().all(|r| r["status"] == "succeeded") && !a.is_empty())
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    println!("RUN LIST: {runs}");
    let rid = runs["runs"][0]["run_id"]
        .as_str()
        .or_else(|| runs[0]["run_id"].as_str())
        .unwrap_or_else(|| panic!("no run in {runs}"))
        .to_owned();
    let run = call(
        &d.engine,
        Method::RunGet,
        serde_json::json!({"run_id": rid}),
    )
    .await;
    assert_eq!(run["status"], "succeeded", "{run}");
    let trace = call(
        &d.engine,
        Method::RunTrace,
        serde_json::json!({"run_id": rid}),
    )
    .await;
    let line = trace
        .to_string()
        .split("},{")
        .find(|l| l.contains("21"))
        .unwrap_or_else(|| panic!("payload missing from the journal: {trace}"))
        .to_owned();
    println!("RUN {rid} JOURNAL LINE: {line}");

    let st = call(&d.engine, Method::SysStatus, serde_json::json!({})).await;
    println!("COUNTS: {}", st["counts"]);
    assert_eq!(st["counts"]["mqtt_received"], 1);
    assert_eq!(st["counts"]["mqtt_published"], 1);
    assert_eq!(st["counts"]["mqtt_dropped"], 0);
    let sub = st["subsystems"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "channel.plant")
        .expect("channel.plant in health");
    assert_eq!(sub["state"], "ok", "{sub}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_non_json_payload_becomes_text_and_a_bad_one_is_counted() {
    let b = Broker::start(false);
    let dir = scratch("mqtt-text-");
    let d = daemon_in(dir.path(), &cfg_for(b.port, "")).unwrap();
    let _h = d.spawn_background();
    for _ in 0..200 {
        if !b.subscribed().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    b.push("sensors/x", &[0xff, 0xfe]);
    b.push("sensors/x", b"on");
    for _ in 0..200 {
        if !b.published().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(
        b.published()[0].payload,
        b"on",
        "text payload passes as text"
    );
    let st = call(&d.engine, Method::SysStatus, serde_json::json!({})).await;
    assert_eq!(st["counts"]["mqtt_dropped"], 1, "{}", st["counts"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dead_broker_is_degraded_in_health_with_the_reason() {
    let port = Broker::free_port();
    let dir = scratch("mqtt-down-");
    let d = daemon_in(dir.path(), &cfg_for(port, "")).unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let st = call(&d.engine, Method::SysStatus, serde_json::json!({})).await;
    let sub = st["subsystems"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "channel.plant")
        .expect("channel.plant");
    println!("HEALTH: {sub}");
    assert_eq!(sub["state"], "degraded");
    assert!(sub["detail"].as_str().unwrap().contains("unreachable"));
}

#[tokio::test(flavor = "multi_thread")]
async fn credentials_never_reach_status_and_a_plaintext_remote_broker_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    let b = Broker::start(true);
    let dir = scratch("mqtt-cred-");
    let pw = dir.path().join("mqtt.pw");
    std::fs::write(&pw, format!("{CANARY}\n")).unwrap();
    std::fs::set_permissions(&pw, std::fs::Permissions::from_mode(0o600)).unwrap();
    let extra = "username = \"x\"\n";
    assert!(
        toml::from_str::<Config>(&cfg_for(b.port, extra)).is_err(),
        "a username value in config is refused: credentials come from env/file"
    );
    let extra = format!(
        "username_env = \"PII_TEST_MQTT_USER_117\"\npassword_file = \"{}\"\n",
        pw.display()
    );
    let d = daemon_in(dir.path(), &cfg_for(b.port, &extra)).unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    let st = call(&d.engine, Method::SysStatus, serde_json::json!({})).await;
    assert!(
        !st.to_string().contains(CANARY),
        "credential in status: {st}"
    );
    assert_eq!(b.connects()[0].password.as_deref(), Some(CANARY));

    let dir2 = scratch("mqtt-refuse-");
    let bad = format!(
        "[[mqtt]]\nname = \"plant\"\nurl = \"mqtt://broker.example.invalid:1883\"\n\
         graph = \"mq\"\npassword_file = \"{}\"\n",
        pw.display()
    );
    let err = daemon_in(dir2.path(), &bad).err().expect("refused");
    println!("REFUSAL: {err}");
    assert!(err.contains("plaintext") && !err.contains(CANARY), "{err}");
}

/// Spec 107's canary, extended to this node: the password reaches the broker
/// (positive control) and nowhere in the trajectory a run wrote.
#[tokio::test(flavor = "multi_thread")]
async fn the_canary_password_is_in_no_trajectory_and_the_control_is_present() {
    use std::os::unix::fs::PermissionsExt;
    let b = Broker::start(false);
    let dir = scratch("mqtt-traj-");
    let pw = dir.path().join("mqtt.pw");
    std::fs::write(&pw, format!("{CANARY}\n")).unwrap();
    std::fs::set_permissions(&pw, std::fs::Permissions::from_mode(0o600)).unwrap();
    let traj = dir.path().join("traj");
    let extra = format!("password_file = \"{}\"\n", pw.display());
    let d = daemon_with(dir.path(), &cfg_for(b.port, &extra), |e| {
        let sink = pii_core::trajectory_sink_with(
            traj.to_str(),
            pii_observe::Retention::default(),
            e.log(),
        )
        .expect("sink");
        e.attach_trajectory(sink);
    })
    .unwrap();
    let _h = d.spawn_background();
    for _ in 0..200 {
        if !b.subscribed().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    b.push("sensors/c", b"marker-117");
    for _ in 0..200 {
        if !b.published().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mut text = String::new();
    for f in std::fs::read_dir(traj.join("trajectories")).unwrap() {
        text.push_str(&std::fs::read_to_string(f.unwrap().path()).unwrap());
    }
    println!(
        "TRAJECTORY canary={} control(marker)={} bytes={}",
        text.matches(CANARY).count(),
        text.matches("marker-117").count(),
        text.len()
    );
    assert_eq!(
        b.connects()[0].password.as_deref(),
        Some(CANARY),
        "sent to the broker"
    );
    assert_eq!(text.matches(CANARY).count(), 0, "canary in trajectory");
    assert!(
        text.matches("marker-117").count() >= 1,
        "control missing: the trajectory is empty"
    );
}
