//! Spec 93 measurement (spec 91 finding 6): does `replay` discarding
//! `apply_completion`'s `Err` (loop budget exceeded) change the resumed run
//! versus the live run? Answer: yes before spec 94, when the budget-exhausting execution is
//! not followed by a loop re-entry (see `replay_matches_live_on_loop_budget_err`, fixed by spec 94).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::PathBuf;
use std::sync::Arc;

use pii_abi::{NodeId, PortName, Terminal};
use pii_core::{Budget, CapSet, Ctx, Observer, RecordingObserver, TraceCtx};
use pii_graph::node::NodeRegistry;
use pii_graph::run;
use pii_graph::spec::{Edge, EdgeKind, GraphLimits, GraphSpec, JoinPolicy, NodeSpec, RetryPolicy};
use pii_store::Store;

fn tmp_root() -> PathBuf {
    let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp"));
    std::fs::create_dir_all(&root).expect("create crate-local _tmp");
    root
}

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

/// Runs `spec` live, then resumes the finished run from its journal.
/// Returns `(live, replayed)` terminals.
fn live_then_replayed(spec: &GraphSpec) -> (Terminal, Terminal) {
    let dir = tempfile::Builder::new()
        .prefix("replay-err-")
        .tempdir_in(tmp_root())
        .expect("tempdir under _tmp");
    let store = Store::open(&dir.path().join("db.redb")).expect("store open");
    let run_id = store.next_run_id().expect("run id");
    let mk_ctx = || {
        Ctx::with_observer(
            CapSet::empty(),
            Budget::unlimited(),
            Arc::new(RecordingObserver::new()) as Arc<dyn Observer>,
            TraceCtx {
                run: run_id.clone(),
                span: 0,
            },
        )
    };
    let reg = NodeRegistry::builtin();
    let input = serde_json::json!({ "exit_codes": [1, 1, 1, 1] });
    let live = run::run(&store, spec, &reg, &mk_ctx(), &run_id, input).expect("run");
    let replayed = run::resume(&store, &reg, &mk_ctx(), &run_id, None).expect("resume");
    (live, replayed)
}

/// Control: with `loop_budget = 3` the budget-exhausting `fix` is followed by a
/// `retry` completion that re-enters `fix`; `iters` was persisted before the
/// `Err`, so the tick loop's dispatch-time budget check re-derives the same
/// failure. The discard is harmless here.
#[test]
fn replay_matches_live_when_budget_err_is_followed_by_reentry() {
    let (live, replayed) = live_then_replayed(&verify_fix_spec(3));
    assert_eq!(live, replayed);
}

/// Spec 94: replay applies the same failure handling as `dispatch::apply`.
///
/// `loop_budget = 1`: `fix` finishes once and `apply_completion` returns
/// `Err("loop_budget exceeded")` BEFORE firing `fix`'s outbound edges. Live,
/// `dispatch::apply` marks the fix->gate edge `Dead`, inserts `fix` into
/// `failed` and sets `failure`. Replay does the same on the `Err`, so the
/// resumed run ends in the identical terminal.
#[test]
fn replay_matches_live_on_loop_budget_err() {
    let (live, replayed) = live_then_replayed(&verify_fix_spec(1));
    assert_eq!(
        live,
        Terminal::Failure {
            error: "loop_budget exceeded for node 'fix'".into()
        }
    );
    assert_eq!(live, replayed);
}
