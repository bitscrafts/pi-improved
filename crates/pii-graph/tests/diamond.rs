//! The four-test diamond suite (pass-3 §4.7; spec F4 test table).
//!
//! Tests A–D run the checked-in example graphs through the full engine
//! (store + journal + scheduler) exactly as the spec's test contract
//! describes: A proves parallel fan-out into one batch, B proves routed
//! dead-branch propagation with a single join, C proves the loud
//! unsatisfiable-join failure, D proves `join="any:cancel"` cancels the loser
//! promptly instead of letting it sleep out its timer.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use pii_abi::{RunEvent, Terminal};
use pii_core::{Budget, CapSet, Ctx, Observer, RecordingObserver, TraceCtx};
use pii_graph::GraphSpec;
use pii_graph::node::NodeRegistry;
use pii_graph::run;
use pii_graph::toml_load::load_toml;
use pii_store::Store;

fn tmp_root() -> PathBuf {
    let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp"));
    std::fs::create_dir_all(&root).expect("create crate-local _tmp");
    root
}

fn ws_root() -> PathBuf {
    // Tests run from crates/pii-graph; examples live at the workspace root.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn load_example(name: &str) -> GraphSpec {
    let full = ws_root().join("examples").join(name);
    let src =
        std::fs::read_to_string(&full).unwrap_or_else(|e| panic!("read {}: {e}", full.display()));
    load_toml(&src).unwrap_or_else(|e| panic!("load {name}: {e}"))
}

/// One engine run of an example graph, with its journal and metrics.
struct RunOut {
    terminal: Terminal,
    journal: Vec<(u64, RunEvent)>,
    obs: Arc<RecordingObserver>,
    wall_ms: u128,
}

fn run_example(name: &str, input: serde_json::Value) -> RunOut {
    run_example_with_validate(name, input, true)
}

/// Like [`run_example`] but with an explicit validation switch, so Test C can
/// exercise the runtime `UnsatisfiableJoin` path on a graph that V6 rejects at
/// load time (pass-3 §4.7 Test C).
fn run_example_with_validate(name: &str, input: serde_json::Value, validate: bool) -> RunOut {
    let spec = load_example(name);
    let dir = tempfile::Builder::new()
        .prefix("diamond-test-")
        .tempdir_in(tmp_root())
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
    let started = Instant::now();
    let terminal =
        run::run_with_validate(&store, &spec, &reg, &ctx, &run_id, input, validate).expect("run");
    let wall_ms = started.elapsed().as_millis();
    let journal = store.read_journal(&run_id).expect("journal");
    RunOut {
        terminal,
        journal,
        obs,
        wall_ms,
    }
}

/// `(at_ms, dur_ms, value)` for every `NodeFinished` of `node`.
fn finished(journal: &[(u64, RunEvent)], node: &str) -> Vec<(u32, u32, serde_json::Value)> {
    journal
        .iter()
        .filter_map(|(_, ev)| match ev {
            RunEvent::NodeFinished {
                node: n,
                value,
                at_ms,
                dur_ms,
                ..
            } if n.as_str() == node => Some((*at_ms, *dur_ms, value.clone())),
            _ => None,
        })
        .collect()
}

fn has_dead(journal: &[(u64, RunEvent)], node: &str) -> bool {
    journal
        .iter()
        .any(|(_, ev)| matches!(ev, RunEvent::NodeDead { node: n } if n.as_str() == node))
}

fn has_cancelled(journal: &[(u64, RunEvent)], node: &str) -> bool {
    journal
        .iter()
        .any(|(_, ev)| matches!(ev, RunEvent::NodeCancelled { node: n } if n.as_str() == node))
}

fn has_failed(journal: &[(u64, RunEvent)], node: &str) -> bool {
    journal
        .iter()
        .any(|(_, ev)| matches!(ev, RunEvent::NodeFailed { node: n, .. } if n.as_str() == node))
}

fn was_started(journal: &[(u64, RunEvent)], node: &str) -> bool {
    journal
        .iter()
        .any(|(_, ev)| matches!(ev, RunEvent::NodeStarted { node: n, .. } if n.as_str() == node))
}

fn tick_widths(journal: &[(u64, RunEvent)]) -> Vec<u32> {
    journal
        .iter()
        .filter_map(|(_, ev)| match ev {
            RunEvent::TickStarted { width, .. } => Some(*width),
            _ => None,
        })
        .collect()
}

/// Test A: the plain diamond drains BOTH arms into one batch (width >= 2),
/// their execution windows overlap, and the join fires exactly once at/after
/// both arms' ends (orki defect 1/2 detectors).
#[test]
fn diamond_plain_runs_both_arms_concurrently() {
    let out = run_example("diamond.toml", serde_json::json!({}));
    assert!(
        matches!(out.terminal, Terminal::Success { .. }),
        "want Success, got {:?}",
        out.terminal
    );
    let a = finished(&out.journal, "a");
    let b = finished(&out.journal, "b");
    assert_eq!(a.len(), 1, "a finished exactly once");
    assert_eq!(b.len(), 1, "b finished exactly once");
    let (a_at, a_dur, _) = a[0];
    let (b_at, b_dur, _) = b[0];
    let overlap = u64::from(a_at) < u64::from(b_at) + u64::from(b_dur)
        && u64::from(b_at) < u64::from(a_at) + u64::from(a_dur);
    assert!(
        overlap,
        "windows must overlap: a [{a_at}..{}] b [{b_at}..{}]",
        a_at + a_dur,
        b_at + b_dur
    );
    let join = finished(&out.journal, "join");
    assert_eq!(join.len(), 1, "join finished exactly once");
    let (j_at, _, _) = join[0];
    assert!(
        j_at >= a_at.saturating_add(a_dur) && j_at >= b_at.saturating_add(b_dur),
        "join.at_ms {j_at} must be >= max(a end, b end)"
    );
    assert!(
        tick_widths(&out.journal).iter().any(|&w| w >= 2),
        "some TickStarted.width must be >= 2, got {:?}",
        tick_widths(&out.journal)
    );
    assert!(
        out.obs.max_metric("graph.batch.width") >= 2,
        "max_metric(graph.batch.width) must be >= 2"
    );
}

/// Test B: the router kills the untaken branch (`NodeDead`, no `NodeFinished`),
/// and the allsettled join fires exactly once keyed by `a` only.
#[test]
fn diamond_routed_marks_branch_dead_and_joins_once() {
    let out = run_example("routed-diamond.toml", serde_json::json!({"branch": "a"}));
    assert!(
        matches!(out.terminal, Terminal::Success { .. }),
        "want Success (must return, not hang), got {:?}",
        out.terminal
    );
    assert!(has_dead(&out.journal, "b"), "b must have a NodeDead event");
    assert!(
        finished(&out.journal, "b").is_empty(),
        "b must never finish"
    );
    let join = finished(&out.journal, "join");
    assert_eq!(join.len(), 1, "join finished exactly once");
    let (_, _, value) = &join[0];
    let obj = value
        .as_object()
        .unwrap_or_else(|| panic!("join value must be an object: {value}"));
    assert!(obj.contains_key("a"), "join value keyed by a: {value}");
    assert!(!obj.contains_key("b"), "join value must lack b: {value}");
}

/// Test C: the statically-unsatisfiable `All` join fails LOUDLY with the
/// exact string, journalled, without ever dispatching the join; the same
/// spec is rejected at lint time with a V6 error.
#[test]
fn diamond_unsatisfiable_join_fails_loudly() {
    // The runtime path is only reachable when validation is skipped (V6
    // rejects this graph at load time); the load-time rejection is asserted
    // at the end of this test.
    let out = run_example_with_validate(
        "unsatisfiable.toml",
        serde_json::json!({"branch": "a"}),
        false,
    );
    match &out.terminal {
        Terminal::Failure { error } => {
            assert!(
                error.contains("join can never be satisfied"),
                "error must name the broken join: {error}"
            );
        }
        other => panic!("want Terminal::Failure, got {other:?}"),
    }
    assert!(
        out.journal.iter().any(|(_, ev)| matches!(
            ev,
            RunEvent::Finished {
                terminal: Terminal::Failure { .. },
                ..
            }
        )),
        "the failure must be journalled as Finished(Failure)"
    );
    assert!(
        !was_started(&out.journal, "join"),
        "join must never be dispatched"
    );
    assert!(
        finished(&out.journal, "join").is_empty(),
        "join must never finish"
    );
    let reg = NodeRegistry::builtin();
    let errs = pii_graph::validate(&load_example("unsatisfiable.toml"), &reg)
        .expect_err("unsatisfiable.toml must fail validation");
    assert!(
        errs.iter().any(|e| e.starts_with("V6")),
        "want a V6 error: {errs:?}"
    );
}

/// Test D: `join="any:cancel"` cancels the losing arm (`NodeCancelled`, NOT
/// `NodeFailed`) and the run ends long before the 5000 ms loser timer expires.
#[test]
fn diamond_race_cancels_loser() {
    let out = run_example("race.toml", serde_json::json!({}));
    assert!(
        matches!(out.terminal, Terminal::Success { .. }),
        "want Success, got {:?}",
        out.terminal
    );
    let use_fin = finished(&out.journal, "use");
    assert_eq!(use_fin.len(), 1, "use finished exactly once");
    let (_, _, value) = &use_fin[0];
    let obj = value
        .as_object()
        .unwrap_or_else(|| panic!("use value must be an object: {value}"));
    assert!(obj.contains_key("fast"), "use value keyed by fast: {value}");
    assert!(
        !obj.contains_key("slow"),
        "use value must lack slow: {value}"
    );
    assert!(
        has_cancelled(&out.journal, "slow"),
        "slow must have a NodeCancelled event"
    );
    assert!(!has_failed(&out.journal, "slow"), "slow must NOT be failed");
    assert!(
        out.wall_ms < 2000,
        "wall {}ms must be < 2000ms although slow.timer_ms == 5000",
        out.wall_ms
    );
}

/// Test E (opus review F8): a multi-node losing arm is cancelled transitively.
/// `fanout -> {slowA -> slowB, fast} -> use(any:cancel)`. When `fast` wins,
/// BOTH `slowA` and `slowB` must be cancelled (not just `slowB`, the direct
/// feeder of `use`), so the run stops paying for the whole losing arm.
#[test]
fn diamond_race_cancels_multi_node_arm() {
    let out = run_example("race-multi-arm.toml", serde_json::json!({}));
    assert!(
        matches!(out.terminal, Terminal::Success { .. }),
        "want Success, got {:?}",
        out.terminal
    );
    let use_fin = finished(&out.journal, "use");
    assert_eq!(use_fin.len(), 1, "use finished exactly once");
    // slowA was running when fast won -> it is cancelled (not failed).
    assert!(
        has_cancelled(&out.journal, "slowA"),
        "slowA must have a NodeCancelled event"
    );
    assert!(
        !has_failed(&out.journal, "slowA"),
        "slowA must NOT be failed"
    );
    // slowB never started (it was waiting for slowA, which was cancelled), so
    // it becomes DEAD via dead propagation, not cancelled.
    assert!(
        has_dead(&out.journal, "slowB"),
        "slowB must have a NodeDead event (its feeder slowA was cancelled)"
    );
    assert!(
        !has_failed(&out.journal, "slowB"),
        "slowB must NOT be failed"
    );
    // The run must end well before the 5000 ms loser timers expire.
    assert!(
        out.wall_ms < 2000,
        "wall {}ms must be < 2000ms although slowA/slowB timer_ms == 5000",
        out.wall_ms
    );
}
