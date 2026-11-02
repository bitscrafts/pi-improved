//! Spec 122: the plugin host end to end through a real daemon — a graph runs
//! a wasm plugin and the journal shows its value; a runaway plugin fails the
//! run with a named timeout; the counters/health reach `sys.status`.

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
const PLUGIN_GRAPH: &str = include_str!("../../../examples/wasm-plugin.toml");
const WRAP_WAT: &str = include_str!("../../../examples/wasm/wrap.wat");

const SPIN_GRAPH: &str = r#"name = "spin"
entry = "spin"

[[node]]
id = "spin"
kind = "wasm"
module = "spin.wasm"
timeout_ms = 200
"#;

const SPIN_WAT: &str = r#"(module (memory (export "memory") 1)
  (func (export "alloc") (param i32) (result i32) (i32.const 1024))
  (func (export "run") (param i32 i32) (result i64) (loop $l (br $l)) (i64.const 0)))"#;

const ESCAPE_GRAPH: &str = r#"name = "escape"
entry = "e"

[[node]]
id = "e"
kind = "wasm"
module = "../outside.wasm"
"#;

fn scratch(p: &str) -> tempfile::TempDir {
    pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), p).expect("tempdir")
}

fn daemon_in(dir: &Path, config_text: &str) -> (Arc<Daemon>, Arc<Recorder>) {
    let hello = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/phase2/hello.json");
    std::fs::create_dir_all(dir.join("ws")).unwrap();
    std::fs::create_dir_all(dir.join("graphs")).unwrap();
    let plugins = dir.join("plugins");
    std::fs::create_dir_all(&plugins).unwrap();
    std::fs::write(plugins.join("wrap.wasm"), wat::parse_str(WRAP_WAT).unwrap()).unwrap();
    std::fs::write(plugins.join("spin.wasm"), wat::parse_str(SPIN_WAT).unwrap()).unwrap();
    for (n, g) in [
        ("wasm-plugin", PLUGIN_GRAPH),
        ("spin", SPIN_GRAPH),
        ("escape", ESCAPE_GRAPH),
    ] {
        std::fs::write(dir.join(format!("graphs/{n}.toml")), g).unwrap();
    }
    let engine = Engine::new(EngineConfig {
        grant_ceiling: None,
        workspace: dir.join("ws"),
        store_path: dir.join("store.redb"),
        model: format!("scripted:{}", hello.display()),
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

async fn call(engine: &Arc<Engine>, m: Method, params: serde_json::Value) -> serde_json::Value {
    let stream = pii_server::engine::loopback::spawn_loopback(Arc::clone(engine));
    let (r, w) = tokio::io::split(stream);
    let info = PeerInfo {
        name: "spec-122".into(),
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
        serde_json::json!({"graph": graph, "input": {"x": 1}}),
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

fn wasm_cfg(dir: &Path) -> String {
    format!(
        "[wasm]\ndir = \"{}\"\ntimeout_ms = 2000\n",
        dir.join("plugins").display()
    )
}

fn wasm_sub(st: &serde_json::Value) -> Option<&serde_json::Value> {
    st["subsystems"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "wasm")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_graph_runs_a_wasm_plugin_and_the_journal_shows_the_returned_value() {
    let dir = scratch("wasm-e2e-");
    let (d, log) = daemon_in(dir.path(), &wasm_cfg(dir.path()));
    let (rid, run, trace) = run_graph(&d.engine, "wasm-plugin").await;
    assert_eq!(run["status"], "succeeded", "{run}");
    let text = trace.to_string();
    println!("WASM PLUGIN — run {rid} journal:");
    for line in text.split("},{") {
        if line.contains("got") {
            println!("  {line}");
        }
    }
    // The plugin wrapped the run input (the engine adds prompt/session around `x`).
    let out = &trace["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["event"]["kind"] == "node_finished")
        .unwrap()["event"]["value"];
    assert_eq!(out["got"]["x"], 1, "{text}");

    let kinds: Vec<_> = log
        .events()
        .into_iter()
        .filter(|e| e.kind.starts_with("wasm."))
        .map(|e| e.kind)
        .collect();
    println!("EVENTS: {kinds:?}");
    assert_eq!(
        kinds,
        [names::WASM_LOAD, names::WASM_INSTANTIATE, names::WASM_CALL]
    );
    let e = &log.events_named(names::WASM_CALL)[0];
    assert!(
        e.fields
            .get("module")
            .unwrap()
            .as_str()
            .unwrap()
            .ends_with("wrap.wasm")
    );
    assert_eq!(e.fields.get("entry"), Some(&serde_json::json!("run")));

    let st = call(&d.engine, Method::SysStatus, serde_json::json!({})).await;
    println!(
        "COUNTS: wasm_calls={} wasm_refusals={}",
        st["counts"]["wasm_calls"], st["counts"]["wasm_refusals"]
    );
    assert_eq!(st["counts"]["wasm_calls"]["ok"], 1);
    assert_eq!(st["counts"]["wasm_refusals"], 0);
    let sub = wasm_sub(&st).expect("wasm in health");
    println!("HEALTH: {sub}");
    assert_eq!(sub["state"], "ok");
    assert_eq!(sub["identity"]["runtime"], "wasmtime 48.0.5");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_runaway_plugin_fails_the_run_with_a_named_timeout_and_moves_the_counter() {
    let dir = scratch("wasm-spin-");
    let (d, log) = daemon_in(dir.path(), &wasm_cfg(dir.path()));
    let (rid, run, trace) = run_graph(&d.engine, "spin").await;
    println!("NEGATIVE runaway plugin — run {rid}: {run}\n  journal: {trace}");
    assert_eq!(run["status"], "failed", "{run}");
    assert!(
        trace.to_string().contains("deadline exceeded after 200ms"),
        "{trace}"
    );
    assert_eq!(log.events_named(names::WASM_TIMEOUT).len(), 1);
    let st = call(&d.engine, Method::SysStatus, serde_json::json!({})).await;
    assert_eq!(st["counts"]["wasm_calls"]["timeout"], 1, "{}", st["counts"]);
    assert_eq!(st["counts"]["wasm_calls"]["ok"], 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_module_path_escaping_the_dir_fails_the_run_and_counts_a_refusal() {
    let dir = scratch("wasm-escape-");
    std::fs::create_dir_all(dir.path()).unwrap();
    let (d, log) = daemon_in(dir.path(), &wasm_cfg(dir.path()));
    std::fs::write(
        dir.path().join("outside.wasm"),
        wat::parse_str(SPIN_WAT).unwrap(),
    )
    .unwrap();
    let (rid, run, trace) = run_graph(&d.engine, "escape").await;
    println!("NEGATIVE escaping path — run {rid}: {run}\n  journal: {trace}");
    assert_eq!(run["status"], "failed", "{run}");
    assert!(
        trace.to_string().contains("outside the configured dir"),
        "{trace}"
    );
    let e = &log.events_named(names::WASM_REFUSAL)[0];
    assert_eq!(
        e.fields.get("reason"),
        Some(&serde_json::json!("out_of_scope"))
    );
    let st = call(&d.engine, Method::SysStatus, serde_json::json!({})).await;
    assert_eq!(st["counts"]["wasm_refusals"], 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn with_no_wasm_section_the_host_is_off_and_a_wasm_graph_fails_loudly() {
    let dir = scratch("wasm-off-");
    let (d, log) = daemon_in(dir.path(), "# no [wasm]\n");
    let (rid, run, trace) = run_graph(&d.engine, "wasm-plugin").await;
    println!("NEGATIVE missing [wasm] — run {rid}: {run}\n  journal: {trace}");
    assert_eq!(run["status"], "failed", "{run}");
    assert!(trace.to_string().contains("not configured"), "{trace}");
    let e = &log.events_named(names::WASM_REFUSAL)[0];
    assert_eq!(
        e.fields.get("reason"),
        Some(&serde_json::json!("not_configured"))
    );
    let st = call(&d.engine, Method::SysStatus, serde_json::json!({})).await;
    assert!(wasm_sub(&st).is_none(), "no wasm subsystem when off");
}

#[test]
fn a_wasm_dir_that_does_not_exist_is_refused_at_daemon_build() {
    let dir = scratch("wasm-bad-");
    std::fs::create_dir_all(dir.path().join("ws")).unwrap();
    let hello = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/phase2/hello.json");
    let engine = Engine::new(EngineConfig {
        grant_ceiling: None,
        workspace: dir.path().join("ws"),
        store_path: dir.path().join("store.redb"),
        model: format!("scripted:{}", hello.display()),
        sandbox: SandboxProfileName::Off,
        route_policy: std::collections::BTreeMap::new(),
        tiers: Vec::new(),
        session_runs_cap: pii_server::DEFAULT_SESSION_RUNS_CAP,
        llama_host: None,
        graph_dir: dir.path().join("graphs"),
        mqtt_tool_url: None,
    })
    .unwrap();
    let p = dir.path().join("pii.toml");
    std::fs::write(&p, "[wasm]\ndir = \"/nonexistent/plugins\"\n").unwrap();
    let cfg = Config::load(&p).unwrap();
    let err = Daemon::build(engine, &cfg, dir.path(), p, Arc::new(pii_core::SystemClock))
        .err()
        .expect("refused");
    assert!(err.to_string().contains("wasm: dir"), "{err}");
}
