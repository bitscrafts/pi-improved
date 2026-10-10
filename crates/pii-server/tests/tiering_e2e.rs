//! Spec 127: tiering end to end through a real daemon. A record written by a run walks
//! hot -> warm (time passes), is read back by a `recall` run (warm -> hot), then goes quiet to
//! cold, while the tier gauges and transition counters move in `sys.status` of the live daemon;
//! then an audited archive removes the cold records and a restore brings them back.

#![allow(
    clippy::print_stdout,
    clippy::disallowed_macros,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::too_many_lines
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use pii_memory::entity::entity_id;
use pii_observe::{Recorder, names};
use pii_proto::method::Method;
use pii_proto::sys::PeerInfo;
use pii_server::daemon::Daemon;
use pii_server::daemon::config::Config;
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;

const REMEMBER: &str = include_str!("../../../examples/memory-remember.toml");
const RECALL: &str = include_str!("../../../examples/memory-recall.toml");

fn scratch(p: &str) -> tempfile::TempDir {
    pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), p).expect("tempdir")
}

fn daemon_in(dir: &Path, config_text: &str) -> (Arc<Daemon>, Arc<Recorder>) {
    let hello = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/phase2/hello.json");
    std::fs::create_dir_all(dir.join("ws")).unwrap();
    std::fs::create_dir_all(dir.join("graphs")).unwrap();
    for (n, g) in [("remember", REMEMBER), ("recall", RECALL)] {
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
        llama_host: None,
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
        name: "spec-127".into(),
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

/// Runs `graph` to a terminal state; returns `(run_id, run, journal)`. A refused start comes back as the error.
async fn run_graph(
    e: &Arc<Engine>,
    graph: &str,
    input: serde_json::Value,
) -> (String, serde_json::Value, serde_json::Value) {
    let r = call(
        e,
        Method::RunStart,
        serde_json::json!({"graph": graph, "input": input}),
    )
    .await;
    let Some(rid) = r["run_id"].as_str().map(str::to_owned) else {
        return (String::new(), r, serde_json::Value::Null);
    };
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

async fn counts(e: &Arc<Engine>) -> serde_json::Value {
    call(e, Method::SysStatus, serde_json::json!({})).await["counts"].clone()
}

fn show(label: &str, c: &serde_json::Value) {
    println!(
        "{label}: tiers={} changes={} archive_ops={} entities={}",
        c["memory_tiers"], c["memory_tier_changes"], c["memory_archive_ops"], c["memory_entities"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_record_walks_hot_warm_hot_cold_through_a_live_daemon_then_archive_and_restore() {
    let dir = scratch("tier-e2e-");
    let (d, log) = daemon_in(
        dir.path(),
        "[memory_graph]\nhot_secs = 1\nwarm_secs = 3\nhot_count = 100\n",
    );
    let (_, run, t) = run_graph(&d.engine, "remember", serde_json::json!({"note": "n"})).await;
    assert_eq!(run["status"], "succeeded", "{run}\n{t}");
    let c = counts(&d.engine).await;
    show("T0 written", &c);
    assert_eq!(
        c["memory_tiers"]["hot"], 3,
        "2 entities + 1 relationship, all hot"
    );

    tokio::time::sleep(Duration::from_millis(2200)).await;
    let c = counts(&d.engine).await;
    show("T1 +2.2s (hot -> warm)", &c);
    assert_eq!(c["memory_tiers"]["warm"], 3);
    assert_eq!(c["memory_tier_changes"]["hot_to_warm"], 3);

    // a later run reads the board and its neighbours: one access restores them
    let board = entity_id("device", "esp32-board");
    let (_, run2, t2) = run_graph(&d.engine, "recall", serde_json::json!({"id": board})).await;
    assert_eq!(run2["status"], "succeeded", "{run2}\n{t2}");
    let c = counts(&d.engine).await;
    show("T2 recall run (warm -> hot)", &c);
    assert_eq!(c["memory_tiers"]["hot"], 3);
    assert_eq!(c["memory_tier_changes"]["warm_to_hot"], 3);

    tokio::time::sleep(Duration::from_millis(2200)).await;
    let c = counts(&d.engine).await;
    show("T3 +2.2s", &c);
    assert_eq!(c["memory_tiers"]["warm"], 3);
    tokio::time::sleep(Duration::from_millis(2500)).await;
    let c = counts(&d.engine).await;
    show("T4 +4.7s (warm -> cold)", &c);
    assert_eq!(c["memory_tiers"]["cold"], 3);
    assert_eq!(c["memory_tier_changes"]["warm_to_cold"], 3);
    assert_eq!(c["memory_entities"], 2, "cold is demoted, not deleted");

    let st = call(&d.engine, Method::SysStatus, serde_json::json!({})).await;
    let sub = st["subsystems"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "memory_entities")
        .expect("memory_entities in health");
    println!("HEALTH: {sub}");
    assert_eq!(sub["identity"]["tier_cold"], "3");
    assert_eq!(sub["identity"]["tier_hot"], "0");

    // the audited removal, against the live daemon's store
    let store = d.engine.entities.get().unwrap();
    let rep = store.archive_before(store.now_secs() - 1).unwrap();
    println!(
        "ARCHIVE: records={} bytes={} file={}",
        rep.records,
        rep.bytes,
        rep.path.as_ref().unwrap().display()
    );
    assert_eq!(rep.records, 3);
    let c = counts(&d.engine).await;
    show("T5 archived", &c);
    assert_eq!(c["memory_entities"], 0);
    assert_eq!(c["memory_archive_ops"]["archived"], 1);

    let back = store.restore(rep.path.as_ref().unwrap()).unwrap();
    println!(
        "RESTORE: restored={} refused={}",
        back.restored,
        back.refused.len()
    );
    assert_eq!((back.restored, back.refused.len()), (3, 0));
    let c = counts(&d.engine).await;
    show("T6 restored", &c);
    assert_eq!(c["memory_entities"], 2);
    assert_eq!(c["memory_tiers"]["hot"], 3);
    assert_eq!(c["memory_archive_ops"]["restored"], 1);

    let kinds: Vec<String> = log.events().into_iter().map(|e| e.kind).collect();
    for n in [
        names::MEMORY_TIER_CHANGED,
        names::MEMORY_ARCHIVED,
        names::MEMORY_RESTORED,
    ] {
        assert!(
            kinds.iter().any(|k| k == n),
            "event {n} missing in {kinds:?}"
        );
    }
}
