//! Spec 276: the coder charges the run's budget with the cost its claude agent reports, caps a claude run by what is
//! left, and refuses to start on an empty budget. A pi coder reports no cost, so it charges nothing and is not bounded.

#![allow(
    non_snake_case,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;
use pii_core::{ArgvScope, Budget, BudgetKind, CapKind, CapSet, Ctx, NullObserver, TraceCtx};
use pii_graph::NodeRegistry;
use pii_observe::{Event, Observer, names};
use pii_server::node_kinds::nodes::coder::budget::{cap_usd, charge_run, start_gate};
use pii_server::node_kinds::nodes::coder::{CoderFactory, agent_binaries};
use pii_server::node_kinds::nodes::criteria;

const ONE_USD: u64 = 1_000_000;

#[derive(Default)]
struct Rec(Mutex<Vec<Event>>);

impl Observer for Rec {
    fn event(&self, e: &Event) {
        self.0.lock().push(e.clone());
    }
}

/// A scratch bin directory: a fake `pi` that exits 0, and a fake `claude` that appends its argv to `argv.log` beside
/// itself and prints one stream-json `result` line whose `total_cost_usd` is `$FAKE_COST` (empty when unset).
fn fake_bin() -> tempfile::TempDir {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "budget-bin-").unwrap();
    let claude = concat!(
        "#!/bin/sh\n",
        "echo \"$*\" >> \"${0%/*}/argv.log\"\n",
        "printf '%s\\n' '{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"num_turns\":1,",
        "\"total_cost_usd\":'\"$FAKE_COST\"',\"result\":\"done\"}'\n",
    );
    for (name, script) in [
        ("pi", String::from("#!/bin/sh\nexit 0\n")),
        ("claude", claude.to_owned()),
    ] {
        let p = dir.path().join(name);
        std::fs::write(&p, script).unwrap();
        std::fs::set_permissions(&p, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    }
    dir
}

/// A context with SPAWN + PROC, `/bin/sh` and the agent binaries under `bin`, and a budget of `micro_usd`.
fn root_ctx(bin: &Path, micro_usd: u64) -> Ctx {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::SPAWN | CapKind::PROC;
    let mut argv0 = vec![String::from("/bin/sh")];
    argv0.extend(agent_binaries(bin));
    argv0.extend(criteria::programs(&BTreeMap::new()));
    caps.proc_argv0 = ArgvScope::new(argv0);
    let trace = TraceCtx {
        run: pii_abi::RunId::from("r000000000276"),
        span: 0,
    };
    Ctx::with_observer(
        caps,
        Budget::new(u64::MAX, u64::MAX, micro_usd, u64::MAX),
        Arc::new(NullObserver),
        trace,
    )
}

/// One node of kind `coder`: `id`, agent `agent`, with the claude credential and the fake cost passed through.
fn coder(id: &str, agent: &str, work: &Path) -> String {
    format!(
        "\n[[node]]\nid = \"{id}\"\nkind = \"coder\"\ntimeout_ms = 60000\ncaps = [\"spawn\", \"proc\"]\nagent = \"{agent}\"\nmodel = \"m-one\"\nbrief = \"do it\"\nspec = \"specs/276-x.md\"\nworkdir = \"{}\"\nenv_pass = [\"CLAUDE_CODE_OAUTH_TOKEN\", \"FAKE_COST\"]\n",
        work.display()
    )
}

/// The graph `code` then `docs`, both of `agent`, with an optional `[limits] max_micro_usd`.
fn two_nodes(work: &Path, agent: &str, limit: Option<u64>) -> String {
    let limits = limit.map_or(String::new(), |m| {
        format!("[limits]\nmax_micro_usd = {m}\n")
    });
    format!(
        "name = \"budget-test\"\nentry = \"code\"\ncaps = [\"spawn\", \"proc\"]\n{limits}{}{}\n[[edge]]\nfrom = \"code\"\nport = \"ok\"\nto = \"docs\"\nkind = \"data\"\n",
        coder("code", agent, work),
        coder("docs", agent, work),
    )
}

/// What one run of the two-node graph left behind.
struct Outcome {
    events: Vec<Event>,
    remaining: u64,
    argv: Vec<String>,
}

/// Runs the two-node graph once with `micro_usd` of budget, the fake claude reporting `cost` as its `total_cost_usd`.
fn run_two(agent: &str, limit: Option<u64>, cost: &str) -> Outcome {
    let bin = fake_bin();
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "budget-work-").unwrap();
    let store_dir =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "budget-store-").unwrap();
    let store = pii_store::Store::open(&store_dir.path().join("store.redb")).unwrap();
    let log = Arc::new(Rec::default());
    let mut reg = NodeRegistry::builtin();
    let env = BTreeMap::from([
        (
            "CLAUDE_CODE_OAUTH_TOKEN".to_owned(),
            "placeholder".to_owned(),
        ),
        ("FAKE_COST".to_owned(), cost.to_owned()),
    ]);
    reg.register(Arc::new(CoderFactory::with_env(
        bin.path().to_path_buf(),
        env,
        log.clone(),
    )));
    let spec =
        pii_graph::load_toml_with_registry(&two_nodes(work.path(), agent, limit), &reg).unwrap();
    let run_id = store.next_run_id().unwrap();
    let ctx = root_ctx(bin.path(), limit.unwrap_or(u64::MAX));
    pii_graph::run::run_with_validate(
        &store,
        &spec,
        &reg,
        &ctx,
        &run_id,
        serde_json::json!({}),
        true,
    )
    .unwrap();
    let argv = std::fs::read_to_string(bin.path().join("argv.log")).unwrap_or_default();
    Outcome {
        events: log.0.lock().clone(),
        remaining: ctx.budget().remaining(BudgetKind::MicroUsd),
        // A brief spans lines, so each spawn is split on the `--session-id` that opens its argv (both nodes are fresh).
        argv: argv
            .split("--session-id ")
            .skip(1)
            .map(|spawn| format!("--session-id {spawn}"))
            .collect(),
    }
}

fn kinds<'a>(events: &'a [Event], kind: &str) -> Vec<&'a Event> {
    events.iter().filter(|e| e.kind == kind).collect()
}

fn field_str(e: &Event, key: &str) -> Option<String> {
    e.fields
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

/// The value after `--max-budget-usd` in one argv line, when the line carries the flag.
fn cap_flag(line: &str) -> Option<String> {
    let words: Vec<&str> = line.split_whitespace().collect();
    words
        .windows(2)
        .find(|w| w[0] == "--max-budget-usd")
        .map(|w| w[1].to_owned())
}

#[test]
fn a_partly_spent_budget_caps_the_next_claude_run() {
    // (a): the first run reports 0.4 USD of a 1.0 USD budget; the second run is capped at the 0.6 that is left.
    let out = run_two("claude", Some(ONE_USD), "0.4");
    assert_eq!(out.argv.len(), 2, "both coders spawned: {:?}", out.argv);
    assert_eq!(
        cap_flag(&out.argv[1]).as_deref(),
        Some("0.6"),
        "{:?}",
        out.argv
    );
    assert_eq!(
        out.remaining, 200_000,
        "1.0 - 0.4 - 0.4 USD after both runs"
    );
}

#[test]
fn an_exhausted_budget_refuses_the_next_coder_and_spawns_nothing() {
    // (b): the first run reports 1.5 USD of a 1.0 USD budget. Its outcome stands; the second never spawns.
    let out = run_two("claude", Some(ONE_USD), "1.5");
    let exited = kinds(&out.events, names::CODER_EXITED);
    assert_eq!(exited.len(), 1, "only the first run exited: {exited:?}");
    assert_eq!(
        field_str(exited[0], "budget_exhausted").as_deref(),
        Some("max_micro_usd"),
        "the run that exhausted the budget says so"
    );
    assert_eq!(
        kinds(&out.events, names::CODER_SPAWNED).len(),
        1,
        "no second spawn"
    );
    assert_eq!(
        out.argv.len(),
        1,
        "the second agent was never started: {:?}",
        out.argv
    );
    // The spec expected the second coder to journal its refusal here. It does not: the engine checks `MicroUsd == 0`
    // before it dispatches the next node and fails the run (pii-graph/src/run/engine.rs:105), so the second coder never
    // runs. The coder's own refusal (`start_gate`) is pinned by the unit test below.
    assert!(
        kinds(&out.events, names::WORKER_START_REFUSED).is_empty(),
        "the engine stops the run before the second coder dispatches"
    );
}

#[test]
fn CONTROL_no_limits_means_no_cap() {
    // (c): without `[limits]` the budget is unlimited: both runs spawn and the second argv has no cap flag.
    let out = run_two("claude", None, "0.4");
    assert_eq!(out.argv.len(), 2, "{:?}", out.argv);
    assert_eq!(cap_flag(&out.argv[1]), None, "{:?}", out.argv);
    assert_eq!(kinds(&out.events, names::WORKER_START_REFUSED).len(), 0);
}

#[test]
fn a_pi_coder_is_not_bounded() {
    // (d): pi reports no cost, so a finite budget is neither charged nor a reason to refuse.
    let out = run_two("pi", Some(ONE_USD), "0.4");
    assert_eq!(
        kinds(&out.events, names::CODER_SPAWNED).len(),
        2,
        "both pi coders ran"
    );
    assert_eq!(out.remaining, ONE_USD, "pi charges nothing");
}

#[test]
fn the_node_cap_wins_when_smaller() {
    // (e): a node cap below what is left is kept.
    let bin = fake_bin();
    let ctx = root_ctx(bin.path(), ONE_USD);
    assert_eq!(cap_usd(&ctx, Some(0.2)), Some(0.2));
    assert_eq!(cap_usd(&ctx, None), Some(1.0), "no node cap: the remainder");
    let unlimited = root_ctx(bin.path(), u64::MAX);
    assert_eq!(
        cap_usd(&unlimited, Some(0.2)),
        Some(0.2),
        "unlimited keeps the node cap"
    );
    assert_eq!(
        cap_usd(&unlimited, None),
        None,
        "unlimited and no node cap: no cap"
    );
}

#[test]
fn an_unusable_cost_charges_nothing() {
    // (f): NaN (not JSON, so absent), a negative cost, and an absent cost all charge nothing.
    let bin = fake_bin();
    for output in [
        r#"{"type":"result","total_cost_usd":NaN}"#,
        r#"{"type":"result","total_cost_usd":-0.5}"#,
        r#"{"type":"result","num_turns":1}"#,
        "not json at all",
    ] {
        let ctx = root_ctx(bin.path(), ONE_USD);
        assert!(!charge_run(&ctx, output), "{output}");
        assert_eq!(
            ctx.budget().remaining(BudgetKind::MicroUsd),
            ONE_USD,
            "{output}"
        );
    }
}

#[test]
fn CONTROL_zero_cost_is_not_exhaustion() {
    // (g): a run that reports 0.0 charges 0 and does not exhaust a 1.0 USD budget.
    let out = run_two("claude", Some(ONE_USD), "0.0");
    assert_eq!(out.remaining, ONE_USD);
    assert_eq!(
        kinds(&out.events, names::CODER_SPAWNED).len(),
        2,
        "both ran"
    );
    for exited in kinds(&out.events, names::CODER_EXITED) {
        assert_eq!(field_str(exited, "budget_exhausted"), None, "{exited:?}");
    }
}

#[test]
fn the_coder_node_itself_refuses_on_an_empty_budget_and_spawns_nothing() {
    // m9: a graph run never reaches this (the engine fails a run on `MicroUsd == 0` before it dispatches a node), so
    // the node's own gate is exercised by building the real coder and calling its `run` directly.
    use pii_abi::NodeInput;
    use pii_graph::node::GraphNode;
    let node_for = |work: &Path| {
        let bin = fake_bin();
        let mut reg = NodeRegistry::builtin();
        let env = BTreeMap::from([
            (
                "CLAUDE_CODE_OAUTH_TOKEN".to_owned(),
                "placeholder".to_owned(),
            ),
            ("FAKE_COST".to_owned(), "0.1".to_owned()),
        ]);
        let log = Arc::new(Rec::default());
        reg.register(Arc::new(CoderFactory::with_env(
            bin.path().to_path_buf(),
            env,
            log.clone(),
        )));
        let spec =
            pii_graph::load_toml_with_registry(&two_nodes(work, "claude", Some(ONE_USD)), &reg)
                .unwrap();
        let cfg = spec
            .nodes
            .iter()
            .find(|n| n.id.to_string() == "code")
            .unwrap()
            .cfg
            .clone();
        let node: Box<dyn GraphNode> = reg.factory("coder").unwrap().build(&cfg).unwrap();
        (node, bin, log)
    };
    let input = NodeInput {
        input: serde_json::json!({}),
        payload: serde_json::json!({}),
        nodes: serde_json::json!({}),
    };
    // Empty budget: the node refuses on its own and nothing is spawned.
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "gate-work-").unwrap();
    let (node, bin, log) = node_for(work.path());
    let out = node.run(&root_ctx(bin.path(), 0), &input).unwrap();
    let port = match out {
        pii_abi::Outcome::Next { port, .. } => port.to_string(),
        other => panic!("expected a port, got {other:?}"),
    };
    assert_eq!(
        port, "err",
        "an empty budget routes the coder to its err port"
    );
    assert!(
        kinds(&log.0.lock(), names::CODER_SPAWNED).is_empty(),
        "nothing was spawned"
    );
    assert!(
        !bin.path().join("argv.log").exists(),
        "the agent was never started"
    );
    // CONTROL: the same node with budget left takes the normal path and spawns.
    let work2 = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "gate-work-").unwrap();
    let (node2, bin2, log2) = node_for(work2.path());
    node2.run(&root_ctx(bin2.path(), ONE_USD), &input).unwrap();
    assert_eq!(
        kinds(&log2.0.lock(), names::CODER_SPAWNED).len(),
        1,
        "a budget with money left still spawns"
    );
}

#[test]
fn start_gate_refuses_only_an_empty_budget() {
    // R1: `Some(reason)` at zero, `None` otherwise.
    let bin = fake_bin();
    assert_eq!(
        start_gate(&root_ctx(bin.path(), 0)).as_deref(),
        Some("run budget exhausted: max_micro_usd")
    );
    assert_eq!(start_gate(&root_ctx(bin.path(), 1)), None);
    assert_eq!(start_gate(&root_ctx(bin.path(), u64::MAX)), None);
}
