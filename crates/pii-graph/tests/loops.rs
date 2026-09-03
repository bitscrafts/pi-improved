//! Loop re-entry tests (spec F4.5, part-2 EC4).
//!
//! A bounded cycle (`loop_budget`) must iterate: a failing `gate` re-enters
//! `fix`, and exceeding the budget is a terminal failure. With pure
//! `transform` nodes the payload is passed through unchanged, so a failing
//! gate always fails — the deterministic proof of re-entry is that `fix`
//! executes exactly `loop_budget` times before the run fails with
//! `loop_budget exceeded`. (The "fails once then passes" success path needs
//! an `agent` node that mutates state, which is exercised at the pii-server
//! level; here we prove the engine's re-entry + reset machinery.)

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::Arc;
use std::time::Instant;

use pii_abi::{NodeId, PortName, RunEvent, Terminal};
use pii_core::{Budget, CapSet, Ctx, Observer, RecordingObserver, TraceCtx};
use pii_graph::node::NodeRegistry;
use pii_graph::run;
use pii_graph::spec::{Edge, EdgeKind, GraphLimits, GraphSpec, JoinPolicy, NodeSpec, RetryPolicy};
use pii_store::Store;

const TMP_ROOT: &str = "/projects/pi-improved/_tmp";

fn node(id: &str, kind: &str, loop_budget: Option<u32>, cfg: serde_json::Value) -> NodeSpec {
    NodeSpec {
        id: NodeId::from(id),
        kind: kind.to_owned(),
        join: JoinPolicy::All,
        loop_budget,
        retry: RetryPolicy::default(),
        caps: 0,
        cfg,
    }
}

fn edge(from: &str, port: &str, to: &str) -> Edge {
    Edge {
        from: NodeId::from(from),
        port: PortName::from(port),
        to: NodeId::from(to),
        kind: EdgeKind::Data,
    }
}

/// `fix -> gate -> {pass: done | fail: retry -> fix}` with `loop_budget` on
/// `fix`. `gate` routes on `$.payload.exit_code`; `fix`/`retry` pass the
/// payload through unchanged, so a non-zero `exit_code` loops forever.
fn verify_fix_spec(loop_budget: u32) -> GraphSpec {
    GraphSpec {
        name: "loops/verify-fix".into(),
        caps: 0,
        nodes: vec![
            node(
                "fix",
                "transform",
                Some(loop_budget),
                serde_json::json!({"expr": "$.payload"}),
            ),
            node(
                "gate",
                "route",
                None,
                serde_json::json!({
                    "on": "$.payload.exit_code",
                    "rules": [
                        {"eq": 0, "port": "pass"},
                        {"port": "fail"}
                    ]
                }),
            ),
            node(
                "done",
                "transform",
                None,
                serde_json::json!({"expr": "$.payload"}),
            ),
            node(
                "retry",
                "transform",
                None,
                serde_json::json!({"expr": "$.payload"}),
            ),
        ],
        edges: vec![
            edge("fix", "ok", "gate"),
            edge("gate", "pass", "done"),
            edge("gate", "fail", "retry"),
            edge("retry", "ok", "fix"),
        ],
        entry: NodeId::from("fix"),
        limits: GraphLimits::default(),
    }
}

struct RunOut {
    terminal: Terminal,
    journal: Vec<(u64, RunEvent)>,
}

fn run_spec(spec: &GraphSpec, input: serde_json::Value) -> RunOut {
    let dir = tempfile::Builder::new()
        .prefix("loops-test-")
        .tempdir_in(TMP_ROOT)
        .expect("tempdir under _tmp");
    let store = Store::open(&dir.path().join("db.redb")).expect("store open");
    let run_id = store.next_run_id().expect("run id");
    let obs = Arc::new(RecordingObserver::new());
    let ctx = Ctx::with_observer(
        CapSet::empty(),
        Budget::unlimited(),
        Arc::clone(&obs) as Arc<dyn Observer>,
        TraceCtx {
            run: run_id.clone(),
            span: 0,
        },
    );
    let reg = NodeRegistry::builtin();
    let _started = Instant::now();
    let terminal = run::run(&store, spec, &reg, &ctx, &run_id, input).expect("run");
    let journal = store.read_journal(&run_id).expect("journal");
    RunOut { terminal, journal }
}

fn finished_count(journal: &[(u64, RunEvent)], node: &str) -> usize {
    journal
        .iter()
        .filter(
            |(_, ev)| matches!(ev, RunEvent::NodeFinished { node: n, .. } if n.as_str() == node),
        )
        .count()
}

/// A fixture that always fails, `loop_budget = 3` → terminal failure with
/// `loop_budget exceeded for node 'fix'`; exactly 3 `fix` executions.
#[test]
fn loop_budget_exceeded_fails() {
    let spec = verify_fix_spec(3);
    // gate always fails (exit_code 1), so the loop never exits.
    let input = serde_json::json!({ "exit_codes": [1, 1, 1, 1] });
    let out = run_spec(&spec, input);
    assert!(
        matches!(&out.terminal, Terminal::Failure { error } if error.contains("loop_budget exceeded for node 'fix'")),
        "{:?}",
        out.terminal
    );
    // fix executed exactly 3 times (initial + 2 re-entries within budget).
    assert_eq!(
        finished_count(&out.journal, "fix"),
        3,
        "fix should run exactly 3 times"
    );
    // gate ran once per completed fix→gate cycle; the 3rd fix run's gate is
    // never reached because the budget blocks before it (fix=3, gate=2).
    assert_eq!(
        finished_count(&out.journal, "gate"),
        2,
        "gate should re-run each completed iteration"
    );
}

/// A larger budget allows more iterations before the terminal failure —
/// proving the budget is honoured, not a fixed cap.
#[test]
fn loop_budget_scales_with_budget() {
    let spec = verify_fix_spec(5);
    let input = serde_json::json!({ "exit_codes": [1, 1, 1, 1, 1, 1] });
    let out = run_spec(&spec, input);
    assert!(
        matches!(&out.terminal, Terminal::Failure { error } if error.contains("loop_budget exceeded for node 'fix'")),
        "{:?}",
        out.terminal
    );
    assert_eq!(finished_count(&out.journal, "fix"), 5);
}
