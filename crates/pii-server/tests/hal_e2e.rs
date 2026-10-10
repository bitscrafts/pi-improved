//! Spec 121: the HAL end to end through a real daemon — a graph blinks a LED
//! on the SIMULATED backend, the refusals fail the run loudly, and the
//! counters/health reach `sys.status`.
//!
//! Every result here is a simulation result. Nothing in this file touches
//! hardware, and nothing on this host can.

#![allow(
    clippy::print_stdout,
    clippy::disallowed_macros,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use pii_observe::{Recorder, names};
use pii_proto::method::Method;
use pii_proto::sys::PeerInfo;
use pii_server::daemon::Daemon;
use pii_server::daemon::config::Config;
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;

/// The shipped example — the file the guide points at is the file executed.
const BLINK: &str = include_str!("../../../examples/hal-blink.toml");

const UNCLAIMED_WRITE: &str = r#"name = "unclaimed"
entry = "on"
caps = ["hw"]

[[node]]
id = "on"
kind = "hal"
op = "write"
pin = 17
level = "high"
"#;

const OUT_OF_SCOPE: &str = r#"name = "oos"
entry = "claim"
caps = ["hw"]

[[node]]
id = "claim"
kind = "hal"
op = "claim"
pin = 18
dir = "out"
"#;

const I2C_READ: &str = r#"name = "i2c"
entry = "claim"
caps = ["hw"]

[[node]]
id = "claim"
kind = "hal"
op = "claim_i2c"
bus = 1
addr = 0x48

[[node]]
id = "id"
kind = "hal"
op = "i2c"
bus = 1
addr = 0x48
write = [0]
read_len = 1

[[edge]]
from = "claim"
port = "ok"
to = "id"
"#;

const HAL_CFG: &str = "[hal]\nbackend = \"sim\"\npins = [17]\ni2c = [\"1:0x48\"]\nspi = [0]\n";

fn scratch(p: &str) -> tempfile::TempDir {
    pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), p).expect("tempdir")
}

fn daemon_in(dir: &Path, config_text: &str) -> (Arc<Daemon>, Arc<Recorder>) {
    let hello = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/phase2/hello.json");
    std::fs::create_dir_all(dir.join("ws")).unwrap();
    std::fs::create_dir_all(dir.join("graphs")).unwrap();
    for (n, g) in [
        ("hal-blink", BLINK),
        ("unclaimed", UNCLAIMED_WRITE),
        ("oos", OUT_OF_SCOPE),
        ("i2c", I2C_READ),
    ] {
        std::fs::write(dir.join(format!("graphs/{n}.toml")), g).unwrap();
    }
    let engine = Engine::new(EngineConfig {
        workspace: dir.join("ws"),
        store_path: dir.join("store.redb"),
        model: format!("scripted:{}", hello.display()),
        sandbox: SandboxProfileName::Off,
        route_policy: std::collections::BTreeMap::new(),
        tiers: Vec::new(),
        session_runs_cap: pii_server::DEFAULT_SESSION_RUNS_CAP,
        graph_dir: dir.join("graphs"),
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

async fn call(engine: &Arc<Engine>, m: Method, params: serde_json::Value) -> serde_json::Value {
    let stream = pii_server::engine::loopback::spawn_loopback(Arc::clone(engine));
    let (r, w) = tokio::io::split(stream);
    let info = PeerInfo {
        name: "spec-121".into(),
        version: "0".into(),
        build: "test".into(),
    };
    let (client, _) = pii_proto::client::Client::connect(r, w, info)
        .await
        .expect("connect");
    let out = client.call(m, params).await;
    client.shutdown();
    out.unwrap_or_else(|e| serde_json::json!({"error": format!("{e:?}")}))
}

/// Runs `graph` to a terminal state; returns `(run_id, run, journal)`.
async fn run_graph(e: &Arc<Engine>, graph: &str) -> (String, serde_json::Value, serde_json::Value) {
    let r = call(
        e,
        Method::RunStart,
        serde_json::json!({"graph": graph, "input": {}}),
    )
    .await;
    let rid = r["run_id"]
        .as_str()
        .unwrap_or_else(|| panic!("{r}"))
        .to_owned();
    for _ in 0..200 {
        let h = call(e, Method::RunGet, serde_json::json!({"run_id": rid})).await;
        if matches!(h["status"].as_str(), Some("succeeded" | "failed")) {
            let t = call(e, Method::RunTrace, serde_json::json!({"run_id": rid})).await;
            return (rid, h, t);
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("run {rid} never terminated");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_graph_blinks_the_simulated_led_and_the_sequence_is_journaled_and_observed() {
    let dir = scratch("hal-blink-");
    let (d, log) = daemon_in(dir.path(), HAL_CFG);
    let (rid, run, trace) = run_graph(&d.engine, "hal-blink").await;
    assert_eq!(run["status"], "succeeded", "{run}");

    println!("SIMULATED BACKEND — run {rid} journal:");
    let text = trace.to_string();
    for line in text.split("},{") {
        if line.contains("level") || line.contains("Failed") {
            println!("  {line}");
        }
    }
    // The values flowed on the ports: high after the first write, low after the second.
    assert!(
        text.contains(r#""op":"read","pin":17,"level":"high""#) || text.contains(r#""high":true"#),
        "{text}"
    );
    assert!(text.contains(r#""high":false"#), "{text}");

    let seq: Vec<String> = log
        .events()
        .into_iter()
        .filter(|e| e.kind.starts_with("hal."))
        .map(|e| {
            format!(
                "{} pin={} level={}",
                e.kind,
                e.fields
                    .get("pin")
                    .map_or(String::new(), ToString::to_string),
                e.fields
                    .get("level")
                    .map_or(String::new(), ToString::to_string),
            )
        })
        .collect();
    println!("EVENTS:\n  {}", seq.join("\n  "));
    assert_eq!(
        seq,
        [
            "hal.claim pin=17 level=",
            "hal.write pin=17 level=\"high\"",
            "hal.read pin=17 level=\"high\"",
            "hal.write pin=17 level=\"low\"",
            "hal.read pin=17 level=\"low\"",
            "hal.release pin=17 level=",
        ]
    );

    let st = call(&d.engine, Method::SysStatus, serde_json::json!({})).await;
    println!("COUNTS: {}", st["counts"]);
    assert_eq!(st["counts"]["hal_ops"]["write"], 2);
    assert_eq!(st["counts"]["hal_ops"]["read"], 2);
    assert_eq!(st["counts"]["hal_ops"]["claim"], 1);
    assert_eq!(st["counts"]["hal_refusals"], 0);
    let sub = st["subsystems"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "hal")
        .expect("hal in health");
    println!("HEALTH: {sub}");
    assert_eq!(sub["state"], "ok");
    assert_eq!(sub["identity"]["backend"], "sim");
    assert_eq!(sub["identity"]["claimed_pins"], "", "released again");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unclaimed_write_fails_the_run_and_moves_the_refusals_counter() {
    let dir = scratch("hal-unclaimed-");
    let (d, log) = daemon_in(dir.path(), HAL_CFG);
    let (rid, run, trace) = run_graph(&d.engine, "unclaimed").await;
    println!("NEGATIVE unclaimed write — run {rid}: {run}\n  journal: {trace}");
    assert_eq!(run["status"], "failed", "{run}");
    assert!(trace.to_string().contains("not claimed"), "{trace}");
    let e = &log.events_named(names::HAL_ERROR)[0];
    assert_eq!(
        e.fields.get("reason"),
        Some(&serde_json::json!("unclaimed"))
    );
    let st = call(&d.engine, Method::SysStatus, serde_json::json!({})).await;
    assert_eq!(st["counts"]["hal_refusals"], 1, "{}", st["counts"]);
    assert_eq!(
        st["counts"]["hal_ops"]["write"], 0,
        "the write never happened"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pin_outside_the_scope_fails_the_run_even_though_the_device_has_it() {
    let dir = scratch("hal-oos-");
    let (d, _) = daemon_in(dir.path(), HAL_CFG);
    let (rid, run, trace) = run_graph(&d.engine, "oos").await;
    println!("NEGATIVE out-of-scope pin — run {rid}: {run}\n  journal: {trace}");
    assert_eq!(run["status"], "failed", "{run}");
    assert!(
        trace.to_string().contains("outside the declared scope"),
        "{trace}"
    );
    let st = call(&d.engine, Method::SysStatus, serde_json::json!({})).await;
    assert_eq!(st["counts"]["hal_refusals"], 1);
    let sub = st["subsystems"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "hal")
        .unwrap();
    assert!(
        sub["identity"]["last_error"]
            .as_str()
            .unwrap()
            .contains("pin 18"),
        "{sub}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn with_no_hal_section_the_hal_is_off_and_a_hal_graph_fails_loudly() {
    let dir = scratch("hal-off-");
    let (d, log) = daemon_in(dir.path(), "# no [hal]\n");
    let (rid, run, trace) = run_graph(&d.engine, "hal-blink").await;
    println!("NEGATIVE missing backend — run {rid}: {run}\n  journal: {trace}");
    assert_eq!(run["status"], "failed", "{run}");
    assert!(trace.to_string().contains("not configured"), "{trace}");
    let e = &log.events_named(names::HAL_ERROR)[0];
    assert_eq!(
        e.fields.get("reason"),
        Some(&serde_json::json!("no_backend"))
    );
    let st = call(&d.engine, Method::SysStatus, serde_json::json!({})).await;
    assert!(
        st["subsystems"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["name"] != "hal"),
        "no hal subsystem when off"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_i2c_transaction_flows_its_bytes_on_the_node_port() {
    let dir = scratch("hal-i2c-");
    let (d, _) = daemon_in(dir.path(), HAL_CFG);
    let (rid, run, trace) = run_graph(&d.engine, "i2c").await;
    assert_eq!(run["status"], "succeeded", "{run}");
    let text = trace.to_string();
    println!(
        "SIMULATED I2C — run {rid}: {}",
        text.split("},{").find(|l| l.contains("read")).unwrap_or("")
    );
    assert!(text.contains(r#""read":[90]"#), "0x5A = 90 in {text}");
}

#[test]
fn an_unknown_backend_or_a_bad_scope_entry_is_refused_at_daemon_build() {
    for (bad, needle) in [
        ("[hal]\nbackend = \"nope\"\n", "unknown backend"),
        ("[hal]\nbackend = \"sim\"\ni2c = [\"zzz\"]\n", "7-bit"),
    ] {
        let dir = scratch("hal-bad-");
        std::fs::create_dir_all(dir.path().join("ws")).unwrap();
        let hello =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/phase2/hello.json");
        let engine = Engine::new(EngineConfig {
            workspace: dir.path().join("ws"),
            store_path: dir.path().join("store.redb"),
            model: format!("scripted:{}", hello.display()),
            sandbox: SandboxProfileName::Off,
            route_policy: std::collections::BTreeMap::new(),
            tiers: Vec::new(),
            session_runs_cap: pii_server::DEFAULT_SESSION_RUNS_CAP,
            graph_dir: dir.path().join("graphs"),
        })
        .unwrap();
        let p = dir.path().join("pii.toml");
        std::fs::write(&p, bad).unwrap();
        let cfg = Config::load(&p).unwrap();
        let err = Daemon::build(engine, &cfg, dir.path(), p, Arc::new(pii_core::SystemClock))
            .err()
            .expect("refused");
        assert!(err.to_string().contains(needle), "{err}");
    }
}
