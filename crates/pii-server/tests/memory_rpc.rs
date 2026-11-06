//! Memory RPC round trip (spec 024 N5, closes C8): `memory.put` →
//! `memory.search` → `memory.consolidate` driven over the SAME
//! `serve_connection` path a real UDS client uses (F2.9's loopback
//! transport), not the `pii-memory` unit-level API. This is the coverage
//! the 2026-09-11 caveats doc found missing: manual evidence promoted to CI.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::PathBuf;
use std::sync::Arc;

use pii_proto::method::Method;
use pii_proto::sys::PeerInfo;
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;

fn engine_in(dir: &std::path::Path) -> Arc<Engine> {
    Engine::new(EngineConfig {
        grant_ceiling: None,
        workspace: dir.to_path_buf(),
        store_path: dir.join("store.redb"),
        model: format!("scripted:{}", fixtures_hello().display()),
        sandbox: SandboxProfileName::Off,
        route_policy: std::collections::BTreeMap::new(),
        tiers: Vec::new(),
        session_runs_cap: pii_server::DEFAULT_SESSION_RUNS_CAP,
        llama_host: None,
        models: pii_server::daemon::empty_model_roles(),
        graph_dir: dir.join("graphs"),
        mqtt_tool_url: None,
    })
    .expect("engine")
}

fn fixtures_hello() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .join("fixtures/phase2/hello.json")
}

fn client_info() -> PeerInfo {
    PeerInfo {
        name: "memory-rpc-test".into(),
        version: "0".into(),
        build: "test".into(),
    }
}

/// `memory.put` → `memory.search` → `memory.consolidate`, exactly the
/// sequence the 2026-09-11 manual evidence exercised, now over the wire
/// protocol instead of the pii-memory API directly.
#[tokio::test]
async fn memory_put_search_consolidate_round_trip() {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "memory-rpc-")
        .expect("tempdir");
    let engine = engine_in(dir.path());
    let stream = pii_server::engine::loopback::spawn_loopback(engine);
    let (r, w) = tokio::io::split(stream);
    let (client, _hello) = pii_proto::client::Client::connect(r, w, client_info())
        .await
        .expect("connect");

    // 1. memory.put
    let put = client
        .call(
            Method::MemoryPut,
            serde_json::json!({"text": "the round trip test writes this exact fact"}),
        )
        .await
        .expect("memory.put");
    let id = put
        .get("id")
        .and_then(|v| v.as_str())
        .expect("put must return an id")
        .to_owned();
    assert_ne!(id, "");

    // 2. memory.search: the doc must come back, WITH its text inline (N3).
    let search = client
        .call(
            Method::MemorySearch,
            serde_json::json!({"q": "round trip test fact", "k": 5}),
        )
        .await
        .expect("memory.search");
    let hits = search
        .get("hits")
        .and_then(|v| v.as_array())
        .expect("hits array");
    assert!(!hits.is_empty(), "search must find the doc just put");
    let hit = hits
        .iter()
        .find(|h| h.get("doc").and_then(|d| d.as_str()) == Some(id.as_str()))
        .unwrap_or_else(|| panic!("hit {id} not found in {hits:?}"));
    assert_eq!(
        hit.get("text").and_then(|v| v.as_str()),
        Some("the round trip test writes this exact fact"),
        "spec 024 N3: the hit must carry the recalled text inline, got {hit:?}"
    );

    // 3. memory.consolidate: a clean run over a single fresh doc merges/
    // archives nothing, and must not error.
    let consolidate = client
        .call(Method::MemoryConsolidate, serde_json::json!({}))
        .await
        .expect("memory.consolidate");
    assert_eq!(
        consolidate
            .get("merged")
            .and_then(serde_json::Value::as_u64),
        Some(0)
    );
    assert_eq!(
        consolidate
            .get("archived")
            .and_then(serde_json::Value::as_u64),
        Some(0)
    );

    client.shutdown();
}

/// Spec 205: the store, not a file beside it, knows what it holds. A put of text the store
/// already has reports `unchanged: true` and the same id; a new text reports no `unchanged`.
#[tokio::test]
async fn a_put_of_text_the_store_already_holds_is_reported_unchanged() {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "memory-unchanged-")
        .expect("tempdir");
    let engine = engine_in(dir.path());
    let stream = pii_server::engine::loopback::spawn_loopback(engine);
    let (r, w) = tokio::io::split(stream);
    let (client, _hello) = pii_proto::client::Client::connect(r, w, client_info())
        .await
        .expect("connect");
    let text = "PII-SPEC 205 | status=no-row | the record is ingested by a verb";

    let first = client
        .call(Method::MemoryPut, serde_json::json!({"text": text}))
        .await
        .expect("first memory.put");
    assert_ne!(
        first.get("unchanged"),
        Some(&serde_json::json!(true)),
        "{first}"
    );

    let second = client
        .call(Method::MemoryPut, serde_json::json!({"text": text}))
        .await
        .expect("second memory.put");
    assert_eq!(
        second.get("unchanged"),
        Some(&serde_json::json!(true)),
        "{second}"
    );
    assert_eq!(second.get("id"), first.get("id"));

    client.shutdown();
}
