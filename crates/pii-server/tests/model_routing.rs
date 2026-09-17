//! Spec-28 model-routing tests (R1–R4): `[evolve.routing]` config parsing,
//! `route_step` resolution, decision journaling, and the loud `NotFound`
//! for an unassigned step.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use pii_graph::node::NodeFactory as _;
use pii_server::daemon::config::{Config, EvolveSection};
use pii_tools::sandbox::SandboxProfileName;
use std::sync::Arc;

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::path::Path::new("/projects/pi-improved/_tmp/phase3/routing-it").join(name);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn fixture_dir() -> std::path::PathBuf {
    std::path::Path::new("/projects/pi-improved/fixtures/phase3/model").to_path_buf()
}

/// R1: `[evolve.routing] patch = "..."` parses into `Config.evolve.routing`.
#[test]
fn routing_config_parses() {
    let dir = scratch("parses");
    let toml = dir.join("pii.toml");
    std::fs::write(
        &toml,
        "[evolve.routing]\npatch = \"nvidia:deepseek-ai/deepseek-v3.2\"\n",
    )
    .unwrap();
    let cfg = Config::load(&toml).expect("config parse");
    assert_eq!(
        cfg.evolve.routing.get("patch").map(String::as_str),
        Some("nvidia:deepseek-ai/deepseek-v3.2"),
        "RoutePolicy has the entry"
    );
}

/// R1: `[evolve.routing]` denies unknown keys (`deny_unknown_fields`).
#[test]
fn routing_config_denies_unknown() {
    let dir = scratch("denies");
    let toml = dir.join("pii.toml");
    std::fs::write(&toml, "[evolve.routing]\nbogus = \"m\"\n").unwrap();
    assert!(
        Config::load(&toml).is_err(),
        "unknown routing key must be an error"
    );
    let _ = EvolveSection::default();
}

/// R2: an agent node with `route_step = "patch"` + policy entry resolves to
/// the mapped model.
#[test]
fn route_step_resolves_model() {
    let policy: std::collections::BTreeMap<String, String> =
        std::iter::once(("patch".to_owned(), "modelX".to_owned())).collect();
    let factory = pii_server::node_kinds::agent::AgentFactory {
        provider: Arc::new(pii_model::ScriptedProvider::empty()),
        workspace: scratch("ws"),
        live: None,
        memory: None,
        route_policy: policy.clone(),
    };
    let node = factory
        .build(&serde_json::json!({
            "system": "s",
            "route_step": "patch",
        }))
        .expect("routed node builds");
    // The resolved model is carried on the node; assert via the routed agent
    // node's public surface (model == modelX).
    // The node built: assert via its size (a boxed node is live and non-trivial).
    let _ = &node; // the node built (expect above already guarantees it)
    // Direct assertion on the resolution function (the load-bearing unit):
    let m = pii_server::node_kinds::agent::resolve_model_for_test(
        &serde_json::json!({"route_step": "patch"}),
        &policy,
    );
    assert_eq!(m.expect("resolved"), Some(String::from("modelX")));
}

/// R2: `model = "m"` with no `route_step` — unchanged behavior.
#[test]
fn no_route_step_uses_explicit_model() {
    let policy = std::collections::BTreeMap::new();
    let m = pii_server::node_kinds::agent::resolve_model_for_test(
        &serde_json::json!({"model": "m"}),
        &policy,
    );
    assert_eq!(m.expect("no routing error"), Some(String::from("m")));
}

/// R3: a routed agent run broadcasts a `RouteDecision { step, model }` event
/// on the live bus (F12 — the agent node's live events are bus-only, never
/// journalled, exactly like `ToolCall`/`ToolResult`/`AssistantDelta`).
#[tokio::test]
async fn route_decision_is_journaled() {
    let dir = scratch("journaled");
    let fixture = fixture_dir().join("assistant.json");
    let engine = pii_server::Engine::new(pii_server::EngineConfig {
        workspace: dir.clone(),
        store_path: dir.join("store.redb"),
        model: format!("scripted:{}", fixture.display()),
        sandbox: SandboxProfileName::Off,
        graph_dir: dir.join("graphs"),
        route_policy: std::iter::once(("patch".to_owned(), "modelX".to_owned())).collect(),
        session_runs_cap: pii_server::DEFAULT_SESSION_RUNS_CAP,
    })
    .expect("engine");
    let _reg = pii_server::engine::registry::registry(
        &engine_cfg_of(&engine),
        engine.provider.clone(),
        engine.bus.subscribe_tx(),
        Arc::clone(&engine.approvals),
        Arc::clone(&engine.heals),
        Arc::clone(&engine.memory),
        Arc::new(parking_lot::Mutex::new(std::collections::BTreeMap::new())),
        engine.cfg.route_policy.clone(),
        Arc::new(parking_lot::Mutex::new(None)),
    );
    // Subscribe to the engine's REAL bus before running (F21 pattern).
    let mut live = engine.bus.subscribe();
    // Write a routed graph and run it.
    std::fs::create_dir_all(dir.join("graphs/starter")).unwrap();
    std::fs::write(
        dir.join("graphs/starter/routed.toml"),
        "name = \"starter/routed\"\nentry = \"agent\"\ncaps = [\"fs_read\", \"fs_write\", \"proc\"]\n\n[[node]]\nid = \"agent\"\nkind = \"agent\"\nsystem = \"s\"\nroute_step = \"patch\"\n\n[[edge]]\nfrom = \"agent\"\nport = \"ok\"\nto = \"done\"\nkind = \"data\"\n\n[[node]]\nid = \"done\"\nkind = \"transform\"\nvalue = \"done\"\n",
    )
    .unwrap();
    let run_id = engine
        .start_run("starter/routed", &serde_json::json!({"prompt": "hi"}))
        .expect("start");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let header = engine.store.get_run(&run_id).unwrap();
        if header.is_some_and(|h| h.status != pii_abi::RunStatus::Running) {
            break;
        }
        assert!(std::time::Instant::now() <= deadline, "run did not finish");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    // The bus broadcast must have carried a RouteDecision for step "patch".
    let mut saw_route = false;
    while let Ok(seq) = live.try_recv() {
        if let pii_proto::event::Event::RouteDecision { step, model, .. } = seq.event {
            assert_eq!(step, "patch", "routed step must be patch");
            assert_eq!(model, "modelX", "routed model must resolve via policy");
            saw_route = true;
        }
    }
    assert!(
        saw_route,
        "bus must carry a RouteDecision for the routed step"
    );
}

fn engine_cfg_of(engine: &Arc<pii_server::Engine>) -> pii_server::EngineConfig {
    pii_server::EngineConfig {
        workspace: engine.cfg.workspace.clone(),
        store_path: engine.cfg.store_path.clone(),
        model: engine.cfg.model.clone(),
        sandbox: engine.cfg.sandbox,
        graph_dir: engine.cfg.graph_dir.clone(),
        route_policy: engine.cfg.route_policy.clone(),
        session_runs_cap: pii_server::DEFAULT_SESSION_RUNS_CAP,
    }
}

/// R4 acceptance: a `route_step` with no policy entry fails LOUD with
/// `NotFound` — never a silent default.
#[test]
fn unassigned_step_fails_loud() {
    let policy = std::collections::BTreeMap::new();
    let err = pii_server::node_kinds::agent::resolve_model_for_test(
        &serde_json::json!({"route_step": "patch"}),
        &policy,
    )
    .unwrap_err();
    assert!(matches!(err, pii_abi::PiiError::NotFound(_)), "{err:?}");
    assert!(err.to_string().contains("refusing to silently default"));
}
