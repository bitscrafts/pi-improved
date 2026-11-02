//! Spec 99: `sys.status` reports IDENTITY — which store, which mounts, which
//! build, which config — so two daemons serving different stores can never
//! look alike. R7's decisive test is `two_daemons_differ_in_identity_fields`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use pii_proto::method::Method;
use pii_proto::sys::{PeerInfo, StatusResult};
use pii_server::daemon::Daemon;
use pii_server::daemon::config::Config;
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;

fn fixtures_hello() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .join("fixtures/phase2/hello.json")
}

/// A daemon rooted entirely under `dir`, with its own store, workspace and
/// config text. Returns the engine and the config text's file path.
fn daemon_in(dir: &Path, config_text: &str) -> (Arc<Engine>, PathBuf) {
    let workspace = dir.join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    let engine = Engine::new(EngineConfig {
        grant_ceiling: None,
        workspace,
        store_path: dir.join("store.redb"),
        model: format!("scripted:{}", fixtures_hello().display()),
        sandbox: SandboxProfileName::Off,
        route_policy: std::collections::BTreeMap::new(),
        tiers: Vec::new(),
        session_runs_cap: pii_server::DEFAULT_SESSION_RUNS_CAP,
        llama_host: None,
        graph_dir: dir.join("graphs"),
        mqtt_tool_url: None,
    })
    .expect("engine");
    let cfg_path = dir.join("pii.toml");
    std::fs::write(&cfg_path, config_text).unwrap();
    let mut cfg = Config::load(&cfg_path).expect("config");
    cfg.core.durable_dir = Some(dir.join("durable").display().to_string());
    Daemon::build(
        Arc::clone(&engine),
        &cfg,
        dir,
        cfg_path.clone(),
        Arc::new(pii_core::SystemClock),
    )
    .expect("daemon");
    (engine, cfg_path)
}

async fn status_json(engine: Arc<Engine>) -> serde_json::Value {
    let stream = pii_server::engine::loopback::spawn_loopback(engine);
    let (r, w) = tokio::io::split(stream);
    let info = PeerInfo {
        name: "health-identity-test".into(),
        version: "0".into(),
        build: "test".into(),
    };
    let (client, _hello) = pii_proto::client::Client::connect(r, w, info)
        .await
        .expect("connect");
    let v = client
        .call(Method::SysStatus, serde_json::json!({}))
        .await
        .expect("sys.status");
    client.shutdown();
    v
}

fn scratch(prefix: &str) -> tempfile::TempDir {
    pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), prefix).expect("tempdir")
}

fn canon(p: &Path) -> String {
    dunce::canonicalize(p).unwrap().display().to_string()
}

fn mount_root(v: &serde_json::Value, name: &str) -> String {
    v["mounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == name)
        .unwrap_or_else(|| panic!("no mount {name} in {}", v["mounts"]))["root"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// THE directive's own test: two daemons, different stores/scopes/configs →
/// the payloads differ IN THE IDENTITY FIELDS (not merely in uptime).
#[tokio::test]
async fn two_daemons_differ_in_identity_fields() {
    let (da, db) = (scratch("hid-a-"), scratch("hid-b-"));
    let (ea, _) = daemon_in(da.path(), "# daemon a\n");
    let (eb, _) = daemon_in(db.path(), "# daemon b, different\n");
    let (a, b) = (status_json(ea).await, status_json(eb).await);

    let (pa, pb) = (
        a["stores"][0]["path"].as_str().unwrap(),
        b["stores"][0]["path"].as_str().unwrap(),
    );
    assert_ne!(pa, pb, "different stores must report different paths");
    assert_eq!(pa, canon(&da.path().join("store.redb")));
    assert_eq!(pb, canon(&db.path().join("store.redb")));
    assert!(a["stores"][0]["bytes"].as_u64().unwrap() > 0);
    assert_eq!(a["stores"][0]["format_version"], 1);

    for name in ["workspace", "channels_scope", "runtime_dir"] {
        assert_ne!(
            mount_root(&a, name),
            mount_root(&b, name),
            "mount {name} must differ"
        );
    }
    assert_eq!(mount_root(&a, "workspace"), canon(&da.path().join("ws")));
    assert_eq!(
        mount_root(&a, "channels_scope"),
        canon(&da.path().join("channels"))
    );
    assert_eq!(mount_root(&b, "runtime_dir"), canon(db.path()));

    let (ha, hb) = (
        a["config"]["sha256"].as_str().unwrap(),
        b["config"]["sha256"].as_str().unwrap(),
    );
    assert_eq!(ha.len(), 64);
    assert_ne!(ha, hb, "different config contents must hash differently");
    assert_eq!(
        a["config"]["path"],
        da.path().join("pii.toml").display().to_string()
    );
}

#[tokio::test]
async fn status_keeps_old_keys_and_reports_build_and_counts() {
    let d = scratch("hid-keys-");
    let (e, _) = daemon_in(d.path(), "");
    let v = status_json(e).await;
    for key in [
        "uptime_ms",
        "runs_active",
        "sessions",
        "sandbox_profile",
        "protocol",
        "rss_kb",
        "workers",
        "listeners",
        "channels",
        "memory",
    ] {
        assert!(v.get(key).is_some(), "pre-existing key {key} missing: {v}");
    }
    assert!(v["memory"].get("docs").is_some());
    assert_eq!(
        v["rss_bytes_per_run"], 0,
        "spec 116: present, and 0 until a window of runs exists: {v}"
    );
    assert_eq!(v["build"]["version"], env!("CARGO_PKG_VERSION"));
    assert_ne!(v["build"]["revision"].as_str().unwrap(), "");
    assert_ne!(v["build"]["target"].as_str().unwrap(), "");
    for k in [
        "runs",
        "journal_events",
        "approvals_pending",
        "cron_entries",
        "memory_docs",
    ] {
        assert!(
            v["counts"][k].is_u64(),
            "counts.{k} missing: {}",
            v["counts"]
        );
    }
    let names: Vec<&str> = v["subsystems"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"store") && names.contains(&"memory") && names.contains(&"net"));
    // The typed wire struct accepts the payload too.
    let _: StatusResult = serde_json::from_value(v).unwrap();
}

/// Spec 105 R1: every store the status can reach reports a real version.
#[tokio::test]
async fn every_store_reports_a_real_format_version() {
    let d = scratch("hid-fmt-");
    let (e, _) = daemon_in(d.path(), "# fmt\n");
    let v = status_json(e).await;
    let stores = v["stores"].as_array().unwrap();
    let names: Vec<_> = stores.iter().map(|s| s["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        [
            "main", "memory", "approval", "heal", "queue", "channel", "timer"
        ]
    );
    for s in stores {
        assert_eq!(
            s["format_version"], 1,
            "store {} has no real version",
            s["name"]
        );
        assert!(s["bytes"].as_u64().unwrap() > 0);
    }
}

/// Asserts the reported store files equal the store files on disk.
fn assert_stores_cover_disk(reported: &[String], dir: &Path) {
    // The durable stores (channel, timer) live under `durable/`, not beside the engine store.
    let mut disk: Vec<String> = [dir.to_path_buf(), dir.join("durable")]
        .iter()
        .flat_map(|d| std::fs::read_dir(d).unwrap())
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| Path::new(n).extension().is_some_and(|x| x == "redb"))
        .collect();
    disk.sort();
    let mut rep = reported.to_vec();
    rep.sort();
    assert_eq!(rep, disk, "health must report exactly the stores on disk");
}

fn reported_files(v: &serde_json::Value) -> Vec<String> {
    v["stores"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            Path::new(s["path"].as_str().unwrap())
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect()
}

/// Spec 106 R5: the reported store set equals the `.redb` files the daemon
/// creates on disk, so a store added later cannot be silently omitted.
#[tokio::test]
async fn health_reports_every_store_file_on_disk() {
    let d = scratch("hid-disk-");
    let (e, _) = daemon_in(d.path(), "# disk\n");
    let v = status_json(e).await;
    assert_stores_cover_disk(&reported_files(&v), d.path());
}

/// Mutation control for R5: a hand-written set missing a store must FAIL.
#[tokio::test]
#[should_panic(expected = "health must report exactly the stores on disk")]
async fn store_set_check_fails_when_a_store_is_omitted() {
    let d = scratch("hid-mut-");
    let (e, _) = daemon_in(d.path(), "# mut\n");
    let v = status_json(e).await;
    let mut files = reported_files(&v);
    files.retain(|f| f != "timer.redb");
    assert_stores_cover_disk(&files, d.path());
}
