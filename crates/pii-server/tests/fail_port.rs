//! Spec 40 T6 (R6): a provider failure routed to the `fail` port is journaled
//! by the existing `NodeFinished` machinery — routing to `"fail"` with a
//! `FailurePayload` value needs no new journal-writing code.
//!
//! Built on pii-graph's run harness (like `crates/pii-graph/tests/diamond.rs`)
//! with the real `AgentFactory` registered on top of the built-in kinds, so a
//! genuine agent node with a forced provider failure runs through the scheduler.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::Arc;

use pii_abi::{Origin, PiiError, ProviderFailure};
use pii_core::{Budget, CapSet, Ctx, RecordingObserver, TraceCtx};
use pii_graph::node::NodeRegistry;
use pii_graph::toml_load::load_toml_with_registry;
use pii_store::Store;

/// A provider that always fails with a `PiiError::Provider` classified
/// `ContextOverflow` (`None` status, local next: no HTTP leg).
struct ContextOverflowProvider;

impl pii_model::ModelProvider for ContextOverflowProvider {
    fn name(&self) -> &'static str {
        "ctx-overflow-fake"
    }

    fn chat(
        &self,
        _ctx: &Ctx,
        _req: &pii_model::ChatRequest,
    ) -> Result<pii_model::ChatResponse, PiiError> {
        Err(PiiError::Provider(ProviderFailure {
            status: None,
            origin: Origin::Provider,
            provider: String::from("llamacpp"),
            message: String::from("llamacpp: prompt too long"),
        }))
    }
}

fn scratch(name: &str) -> std::path::PathBuf {
    // Per-invocation: a fixed dir meant two concurrent runs of this binary opened the same redb and the
    // second died with `Database already open` (spec 134; measured 6 of 6 concurrent rounds). The pid is
    // unique because a concurrent run is always another process.
    let dir = std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/_tmp/phase3/fail-port-it"
    ))
    .join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// R6 (spec 40): the existing journaling mechanism carries a `fail`-port
/// outcome without any new journal code. Run a one-node agent graph whose
/// provider is forced to a `ContextOverflow` failure, then read the journal
/// back and assert the `NodeFinished` event shows `port == "fail"` and a
/// value whose `reason` is `ContextOverflow`.
#[test]
fn fail_port_outcome_is_journaled() {
    let dir = scratch("journaled");

    // Registry: built-in kinds + the real agent factory with the failing
    // provider. Loading via `load_toml_with_registry` so the agent node's
    // caps default from this registry (matching what will actually run).
    let mut registry = NodeRegistry::builtin();
    registry.register(Arc::new(pii_server::node_kinds::agent::AgentFactory {
        provider: Arc::new(ContextOverflowProvider),
        route_policy: std::collections::BTreeMap::new(),
        tiers: None,
        models: Arc::default(),
        workspace: dir.clone(),
        live: None,
        memory: None,
    }));
    let spec = graph_spec(&registry);

    let store = Store::open(&dir.join("store.redb")).unwrap();
    let run_id = store.next_run_id().unwrap();
    let obs = Arc::new(RecordingObserver::new());
    let ctx = Ctx::with_observer(
        CapSet::empty(),
        Budget::unlimited(),
        Arc::clone(&obs) as Arc<dyn pii_core::Observer>,
        TraceCtx {
            run: run_id.clone(),
            span: 0,
        },
    );

    let terminal = pii_graph::run::run_with_validate(
        &store,
        &spec,
        &registry,
        &ctx,
        &run_id,
        serde_json::json!({}),
        true,
    )
    .expect("run");

    // The agent routes to `fail` (an untaken port in this stub graph) and the
    // scheduler terminates the run as success without a hard failure.
    assert!(
        matches!(terminal, pii_abi::Terminal::Success { .. }),
        "expected a successful (routed, not crashed) run, got {terminal:?}"
    );

    let journal = store.read_journal(&run_id).unwrap();
    let finished: Vec<_> = journal
        .iter()
        .filter_map(|(_, ev)| match ev {
            pii_abi::RunEvent::NodeFinished {
                node, port, value, ..
            } => Some((
                node.as_str().to_string(),
                port.as_str().to_string(),
                value.clone(),
            )),
            _ => None,
        })
        .collect();
    assert!(
        finished
            .iter()
            .any(|(node, port, _)| node == "agent" && port == "fail"),
        "journal must contain an agent NodeFinished on the fail port, got: {finished:?}"
    );
    let (_, _, value) = finished
        .iter()
        .find(|(node, port, _)| node == "agent" && port == "fail")
        .expect("fail-port event present");
    assert_eq!(
        value.get("reason").and_then(serde_json::Value::as_str),
        Some("context_overflow"),
        "fail-port value must carry reason context_overflow, got: {value}"
    );
}

fn graph_spec(registry: &NodeRegistry) -> pii_graph::GraphSpec {
    let toml = r#"
name = "starter/fail-port-stub"
entry = "agent"
caps = ["fs_read", "fs_write", "proc"]

[[node]]
id = "agent"
kind = "agent"
system = "s"
"#;
    load_toml_with_registry(toml, registry).expect("graph toml loads")
}
