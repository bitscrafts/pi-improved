//! Spec 222 R2: a graph name that no search dir holds is refused with every dir searched and the
//! number of graphs in each. A graph beside the store is found by the same search.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;

const GRAPH: &str = "name = \"coder-demo\"\nentry = \"step\"\ncaps = []\n\n[[node]]\nid = \"step\"\nkind = \"transform\"\nvalue = \"ok\"\n";

/// An engine whose primary graph dir is empty and whose store sits beside `state/graphs`.
fn engine_over(root: &std::path::Path) -> std::sync::Arc<Engine> {
    let workspace = root.join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(root.join("graphs")).unwrap();
    let fixture = root.join("fixture.json");
    std::fs::write(&fixture, r#"{"turns":[]}"#).unwrap();
    Engine::new(EngineConfig {
        grant_ceiling: None,
        workspace,
        store_path: root.join("state/store.redb"),
        model: format!("scripted:{}", fixture.display()),
        sandbox: SandboxProfileName::Off,
        graph_dir: root.join("graphs"),
        route_policy: std::collections::BTreeMap::new(),
        tiers: Vec::new(),
        session_runs_cap: pii_server::DEFAULT_SESSION_RUNS_CAP,
        llama_host: None,
    })
    .unwrap()
}

#[test]
fn unresolved_graph_names_the_directories() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    let beside = root.join("state/graphs");
    std::fs::create_dir_all(&beside).unwrap();
    std::fs::write(beside.join("coder-demo.toml"), GRAPH).unwrap();
    std::fs::write(beside.join("other.toml"), GRAPH).unwrap();
    let engine = engine_over(root);

    let err = engine.load_spec("nope").unwrap_err().to_string();
    let primary = root.join("graphs");
    let expected = format!(
        "searched {} (0) {} (2)",
        primary.display(),
        beside.display()
    );
    assert!(err.contains(&expected), "{err}");
    assert!(err.contains("graph 'nope'"), "{err}");
}

#[test]
fn a_graph_beside_the_store_loads_by_name() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    let beside = root.join("state/graphs");
    std::fs::create_dir_all(&beside).unwrap();
    std::fs::write(beside.join("coder-demo.toml"), GRAPH).unwrap();
    let engine = engine_over(root);

    let spec = engine.load_spec("coder-demo").unwrap();
    assert_eq!(spec.name, "coder-demo");
}
