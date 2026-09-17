//! Spec 41 (Phase 5.1 Track A4) R5/T1–T3: graph-level proof that the real,
//! checked-in `graphs/starter/failover.toml` composes `route`+`retry`
//! against the `fail` port's `FailurePayload` shape the way 026 describes —
//! a `RetrySame` reason retries the same provider, a `NextProvider` reason
//! advances the chain without wasting a retry, and a `Compress` reason
//! never advances the chain at all.
//!
//! Built on the same harness `tests/fail_port.rs` (spec 40) established:
//! `NodeRegistry::builtin()` plus the real `AgentFactory`, loaded through
//! `load_toml_with_registry` so agent caps default the way a live daemon
//! would resolve them.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use pii_abi::{Origin, PiiError, ProviderFailure, RunEvent, Terminal};
use pii_core::{Budget, CapSet, Ctx, RecordingObserver, TraceCtx};
use pii_graph::node::NodeRegistry;
use pii_graph::toml_load::load_toml_with_registry;
use pii_model::{ChatResponse, ModelProvider};
use pii_store::Store;

const FAILOVER_TOML_PATH: &str = "/projects/pi-improved/graphs/starter/failover.toml";

fn scratch(name: &str) -> std::path::PathBuf {
    let dir =
        std::path::Path::new("/projects/pi-improved/_tmp/phase5/failover-graph-it").join(name);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn ok_response() -> ChatResponse {
    ChatResponse {
        text: String::from("done"),
        tool_calls: Vec::new(),
        tokens_in: 1,
        tokens_out: 1,
        micro_usd: 0,
    }
}

/// Fails `primary`'s provider on its first N calls with a fixed error, then
/// succeeds on every call after that. `secondary` always succeeds (its own
/// call count is tracked separately so tests can assert whether it was
/// entered at all).
struct LadderProvider {
    primary_fail_error: Option<PiiError>,
    /// How many of `primary`'s calls should fail before it succeeds.
    /// `usize::MAX` means "always fail" (used by the T3 Compress case,
    /// which must never succeed since Compress is a dead end anyway).
    primary_fails_for: usize,
    primary_calls: AtomicUsize,
    secondary_calls: AtomicUsize,
}

impl LadderProvider {
    fn new(primary_fail_error: PiiError, primary_fails_for: usize) -> Self {
        Self {
            primary_fail_error: Some(primary_fail_error),
            primary_fails_for,
            primary_calls: AtomicUsize::new(0),
            secondary_calls: AtomicUsize::new(0),
        }
    }
}

impl ModelProvider for LadderProvider {
    fn name(&self) -> &'static str {
        "ladder-fake"
    }

    fn chat(&self, ctx: &Ctx, req: &pii_model::ChatRequest) -> Result<ChatResponse, PiiError> {
        // Distinguish primary vs secondary by the node id carried on the
        // context's principal — both share this one provider instance,
        // exactly like the real daemon shares one provider across nodes.
        let node = match ctx.principal() {
            pii_core::Principal::GraphNode { node, .. } => node.as_str().to_owned(),
            _ => String::new(),
        };
        let _ = req;
        if node == "primary" {
            let n = self.primary_calls.fetch_add(1, Ordering::SeqCst);
            if n < self.primary_fails_for {
                let err = self
                    .primary_fail_error
                    .clone()
                    .unwrap_or_else(|| PiiError::Invalid(String::from("no error configured")));
                return Err(err);
            }
            return Ok(ok_response());
        }
        if node == "secondary" {
            self.secondary_calls.fetch_add(1, Ordering::SeqCst);
            return Ok(ok_response());
        }
        Ok(ok_response())
    }
}

fn graph_spec(registry: &NodeRegistry) -> pii_graph::GraphSpec {
    let toml = std::fs::read_to_string(FAILOVER_TOML_PATH)
        .expect("real graphs/starter/failover.toml must exist");
    load_toml_with_registry(&toml, registry).expect("failover.toml loads")
}

fn run_ladder(dir_name: &str, provider: Arc<LadderProvider>) -> (Terminal, Vec<(String, String)>) {
    let dir = scratch(dir_name);
    let mut registry = NodeRegistry::builtin();
    registry.register(Arc::new(pii_server::node_kinds::agent::AgentFactory {
        provider,
        route_policy: std::collections::BTreeMap::new(),
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

    let journal = store.read_journal(&run_id).unwrap();
    let finished: Vec<(String, String)> = journal
        .iter()
        .filter_map(|(_, ev)| match ev {
            RunEvent::NodeFinished { node, port, .. } => {
                Some((node.as_str().to_string(), port.as_str().to_string()))
            }
            _ => None,
        })
        .collect();
    (terminal, finished)
}

/// T1 (spec 41): `primary` fails its first two calls (the initial attempt
/// and `AgentNode`'s own single in-node retry for a `RetrySame` reason —
/// see `agent.rs`'s `is_retryable` retry-once path) with an `Overloaded`
/// (503 -> `RetrySame`) reason, then succeeds on the third call — the
/// GRAPH-level retry driven by `route_failure` -> `retry_primary` ->
/// `primary`'s `loop_budget`. The run must reach `done`, and the journal
/// must show `primary` both fail once (fail port) and finish ok once (after
/// the graph-level retry).
#[test]
fn retry_same_reason_retries_primary_then_succeeds() {
    let err = PiiError::Provider(ProviderFailure {
        status: Some(503),
        origin: Origin::Provider,
        provider: String::from("ollama"),
        message: String::from("server overloaded"),
    });
    assert!(err.is_retryable(), "503 is retryable at the classify layer");
    let provider = Arc::new(LadderProvider::new(err, 2));
    let (terminal, finished) = run_ladder("t1", provider);

    assert!(
        matches!(terminal, Terminal::Success { .. }),
        "expected success, got {terminal:?}"
    );
    let primary_ok_count = finished
        .iter()
        .filter(|(n, p)| n == "primary" && p == "ok")
        .count();
    assert_eq!(
        primary_ok_count, 1,
        "primary must finish 'ok' exactly once (after the retry), got: {finished:?}"
    );
    let primary_fail_count = finished
        .iter()
        .filter(|(n, p)| n == "primary" && p == "fail")
        .count();
    assert_eq!(
        primary_fail_count, 1,
        "primary must finish 'fail' exactly once (the first attempt), got: {finished:?}"
    );
    assert!(
        finished.iter().any(|(n, p)| n == "done" && p == "ok"),
        "run must reach done, got: {finished:?}"
    );
}

/// T2 (spec 41): `primary` always fails with an `Auth` (401 ->
/// `NextProvider`) reason; `secondary` succeeds. The run must reach `done`
/// via `secondary`, with `primary` entered exactly once (no retry wasted on
/// a `NextProvider` failure) and `secondary` entered exactly once.
#[test]
fn next_provider_reason_advances_to_secondary_without_retry() {
    let err = PiiError::Provider(ProviderFailure {
        status: Some(401),
        origin: Origin::Provider,
        provider: String::from("ollama"),
        message: String::from("invalid api key"),
    });
    assert!(!err.is_retryable(), "401 must not be retryable");
    let provider = Arc::new(LadderProvider::new(err, usize::MAX));
    let (terminal, finished) = run_ladder("t2", provider);

    assert!(
        matches!(terminal, Terminal::Success { .. }),
        "expected success, got {terminal:?}"
    );
    let primary_calls = finished.iter().filter(|(n, _)| n == "primary").count();
    assert_eq!(
        primary_calls, 1,
        "primary must be entered exactly once (no retry against NextProvider), got: {finished:?}"
    );
    let secondary_ok_count = finished
        .iter()
        .filter(|(n, p)| n == "secondary" && p == "ok")
        .count();
    assert_eq!(
        secondary_ok_count, 1,
        "secondary must finish 'ok' exactly once, got: {finished:?}"
    );
    assert!(
        finished.iter().any(|(n, p)| n == "done" && p == "ok"),
        "run must reach done via secondary, got: {finished:?}"
    );
}

/// T3 (spec 41, the 026 acceptance test): `primary` always fails with a
/// `ContextOverflow` (`None` status, "prompt too long" -> `Compress`)
/// reason. The run must reach `compress` and must NOT reach `done` —
/// `Compress` never advances the chain (R3). This is the regression guard
/// against routing `compress` onward to `secondary` "to be safe".
#[test]
fn compress_reason_never_advances_the_chain() {
    let err = PiiError::Provider(ProviderFailure {
        status: None,
        origin: Origin::Provider,
        provider: String::from("llamacpp"),
        message: String::from("llamacpp: prompt too long"),
    });
    let provider = Arc::new(LadderProvider::new(err, usize::MAX));
    let (terminal, finished) = run_ladder("t3", provider);

    // The run terminates (compress is a dead end, not a hang): a
    // no-progress termination is the expected shape here, not a panic.
    let _ = &terminal;
    assert!(
        finished.iter().any(|(n, _)| n == "compress"),
        "run must reach compress, got: {finished:?}"
    );
    assert!(
        !finished.iter().any(|(n, _)| n == "done"),
        "run must NEVER reach done via a Compress failure, got: {finished:?}"
    );
    assert!(
        !finished.iter().any(|(n, _)| n == "secondary"),
        "compress must not silently advance the chain to secondary, got: {finished:?}"
    );
}
