//! Spec 423 (an agent node resolves the model role it names, or refuses it by name): an `agent` node that says
//! `model = "@<role>"` takes the role's model and provider from the daemon's `[models.<role>]` table, an unknown role
//! is refused by name before any model call, an explicit model id is passed through, and a failing role call names
//! the role. The node is built through the daemon's registry (`engine::registry::registry`), the same path the
//! channel-started runs take, so the role table reaches it the way it reaches the coder node.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;

use pii_abi::{NodeInput, Outcome, PiiError};
use pii_core::{Budget, CapKind, CapSet, Ctx, PathScope};
use pii_graph::node::NodeFactory;
use pii_model::{ChatRequest, ChatResponse, ModelProvider};
use pii_observe::NullObserver;
use pii_server::daemon::ModelRole;
use pii_server::engine::bus::Bus;
use pii_server::engine::registry::registry;
use pii_server::tiers::TierTable;
use pii_server::{DEFAULT_SESSION_RUNS_CAP, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;

/// The run's default provider: replies `DEFAULT` and records the model id of every request it is asked to serve.
struct Recording {
    seen: Arc<Mutex<Vec<String>>>,
}

impl ModelProvider for Recording {
    fn name(&self) -> &'static str {
        "default"
    }
    fn chat(&self, _ctx: &Ctx, req: &ChatRequest) -> Result<ChatResponse, PiiError> {
        self.seen.lock().push(req.model.clone());
        Ok(ChatResponse {
            text: String::from("DEFAULT"),
            tool_calls: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
            micro_usd: 0,
        })
    }
}

/// Writes a one-turn scripted fixture replying `text`, or an empty one (every call fails); returns its spec.
fn fixture(dir: &Path, name: &str, turns: &serde_json::Value) -> String {
    let p = dir.join(format!("{name}.json"));
    std::fs::write(&p, serde_json::json!({ "turns": turns }).to_string()).unwrap();
    format!("scripted:{}", p.display())
}

fn reply(text: &str) -> serde_json::Value {
    serde_json::json!([{ "text": text, "tool_calls": [] }])
}

fn role(model: &str, provider: &str) -> ModelRole {
    ModelRole {
        model: model.to_owned(),
        provider: provider.to_owned(),
    }
}

/// The agent factory the daemon registers, built with `roles` as the operator's `[models]` table.
fn agent_factory(
    dir: &Path,
    roles: BTreeMap<String, ModelRole>,
    default: Arc<dyn ModelProvider>,
) -> Arc<dyn NodeFactory> {
    let cfg = EngineConfig {
        workspace: dir.to_path_buf(),
        store_path: dir.join("store.redb"),
        model: format!("scripted:{}", dir.join("default.json").display()),
        sandbox: SandboxProfileName::Off,
        graph_dir: dir.join("graphs"),
        route_policy: BTreeMap::new(),
        grant_ceiling: None,
        tiers: Vec::new(),
        models: roles,
        session_runs_cap: DEFAULT_SESSION_RUNS_CAP,
        llama_host: None,
        mqtt_tool_url: None,
    };
    let store = |name: &str| dir.join(name);
    let reg = registry(
        &cfg,
        default,
        Bus::new().subscribe_tx(),
        Arc::new(pii_store::approval::ApprovalStore::open_in_memory().unwrap()),
        Arc::new(pii_store::heal::HealStore::open_in_memory().unwrap()),
        Arc::new(pii_store::MemStore::open(&store("memory.redb")).unwrap()),
        Arc::new(Mutex::new(BTreeMap::new())),
        BTreeMap::new(),
        Arc::new(Mutex::new(None)),
        Arc::new(NullObserver),
        Arc::new(TierTable::default()),
        PathBuf::from("podman"),
    );
    Arc::clone(reg.factory("agent").unwrap())
}

fn ctx(dir: &Path) -> Ctx {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::FS_READ | CapKind::FS_WRITE | CapKind::PROC | CapKind::NET;
    caps.fs_read = PathScope::new(vec![dir.to_path_buf()]);
    caps.fs_write = PathScope::new(vec![dir.to_path_buf()]);
    Ctx::root(caps, Budget::unlimited())
}

fn input() -> NodeInput {
    NodeInput {
        input: serde_json::json!({}),
        payload: serde_json::json!({ "prompt": "hi" }),
        nodes: serde_json::json!({}),
    }
}

/// An agent node as `build_nodes` hands it over: its graph id is injected as `node-id`.
fn node_cfg(id: &str, model: &str) -> serde_json::Value {
    serde_json::json!({ "system": "s", "model": model, "node-id": id })
}

/// Builds the node and runs it once; a refusal at build time and a failure at run time both come back as `Err`.
fn build_and_run(
    factory: &dyn NodeFactory,
    cfg: &serde_json::Value,
    dir: &Path,
) -> Result<Outcome, PiiError> {
    let node = factory.build(cfg)?;
    node.run(&ctx(dir), &input())
}

#[test]
fn an_agent_node_resolves_the_role_it_names() {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "agent-roles-").unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let default = Arc::new(Recording {
        seen: Arc::clone(&seen),
    });
    let roles = BTreeMap::from([(
        String::from("plan"),
        role(
            "fast-model",
            &fixture(dir.path(), "plan", &reply("FROM-ROLE")),
        ),
    )]);
    let factory = agent_factory(dir.path(), roles, default);
    let out = build_and_run(&*factory, &node_cfg("planner", "@plan"), dir.path())
        .unwrap_or_else(|e| panic!("the run did not succeed: {e}"));
    let Outcome::Next { port, value } = &out else {
        panic!("expected an outcome on a port: {out:?}")
    };
    assert_eq!(port.as_str(), "ok", "{value}");
    assert!(
        value.to_string().contains("FROM-ROLE"),
        "the role's provider answered: {value}"
    );
    assert!(
        seen.lock().is_empty(),
        "the default provider was called for a role: {:?}",
        seen.lock()
    );
}

#[test]
#[allow(non_snake_case)] // the name states what it controls for, as the spec names it
fn CONTROL_an_unknown_role_is_refused_by_name() {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "agent-roles-").unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let default = Arc::new(Recording {
        seen: Arc::clone(&seen),
    });
    let factory = agent_factory(dir.path(), BTreeMap::new(), default);
    let err = match build_and_run(&*factory, &node_cfg("planner", "@plan"), dir.path()) {
        Err(e) => e.to_string(),
        Ok(out) => panic!("an unknown role ran: {out:?}"),
    };
    assert!(err.contains("planner"), "names the node id: {err}");
    assert!(err.contains("@plan"), "names the role: {err}");
    assert!(
        seen.lock().is_empty(),
        "a model was called for an unknown role: {:?}",
        seen.lock()
    );
}

#[test]
#[allow(non_snake_case)] // the name states what it controls for, as the spec names it
fn CONTROL_an_explicit_model_id_is_passed_through() {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "agent-roles-").unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let default = Arc::new(Recording {
        seen: Arc::clone(&seen),
    });
    let roles = BTreeMap::from([(
        String::from("plan"),
        role("other-model", "scripted:/does/not/matter.json"),
    )]);
    let factory = agent_factory(dir.path(), roles, default);
    build_and_run(&*factory, &node_cfg("planner", "fast-model"), dir.path())
        .unwrap_or_else(|e| panic!("the explicit model did not run: {e}"));
    assert_eq!(
        *seen.lock(),
        vec![String::from("fast-model")],
        "the provider receives the id unchanged"
    );
}

#[test]
#[allow(non_snake_case)] // the name states what it controls for, as the spec names it
fn CONTROL_a_failing_role_call_names_the_role() {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "agent-roles-").unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let default = Arc::new(Recording {
        seen: Arc::clone(&seen),
    });
    // An empty fixture: the role's provider has no turn to give, so its call fails.
    let roles = BTreeMap::from([(
        String::from("plan"),
        role(
            "fast-model",
            &fixture(dir.path(), "dead", &serde_json::json!([])),
        ),
    )]);
    let factory = agent_factory(dir.path(), roles, default);
    let failure = match build_and_run(&*factory, &node_cfg("planner", "@plan"), dir.path()) {
        Err(e) => e.to_string(),
        Ok(Outcome::Next { port, value }) if port.as_str() != "ok" => value.to_string(),
        Ok(out) => panic!("the failing role call succeeded: {out:?}"),
    };
    assert!(
        failure.contains("plan"),
        "the failure names the role: {failure}"
    );
    assert!(
        seen.lock().is_empty(),
        "the default provider answered for a failing role: {:?}",
        seen.lock()
    );
}
