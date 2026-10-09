//! Spec 103 R4/R5: a real graph run through the `TrajectoryBridge` and the
//! file sink records the engine's terminal verbatim — a FAILED run records
//! `failure`, never `success`, and a successful run records no failure.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use pii_core::{Budget, CapSet, Ctx, Observer, TraceCtx, TrajectoryBridge};
use pii_graph::node::NodeRegistry;
use pii_graph::run;
use pii_graph::toml_load::load_toml;
use pii_observe::{NoRedaction, TrajectorySink};
use pii_store::Store;
use serde_json::Value;

/// Runs example `name` with the sink at `root`; returns the trajectory lines.
fn run_to_trajectory(name: &str, input: Value, validate: bool, root: &Path) -> Vec<Value> {
    let ws = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let spec = load_toml(&std::fs::read_to_string(ws.join("examples").join(name)).unwrap())
        .expect("load example");
    let scratch = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp"));
    std::fs::create_dir_all(&scratch).unwrap();
    let dir = tempfile::Builder::new()
        .prefix("trajectory-")
        .tempdir_in(scratch)
        .unwrap();
    let store = Store::open(&dir.path().join("db.redb")).unwrap();
    let run_id = store.next_run_id().unwrap();
    let sink = Arc::new(TrajectorySink::new(root, Arc::new(NoRedaction)));
    let ctx = Ctx::with_observer(
        CapSet::empty(),
        Budget::unlimited(),
        Arc::new(TrajectoryBridge::new(sink, &run_id, None)) as Arc<dyn Observer>,
        TraceCtx {
            run: run_id.clone(),
            span: 0,
        },
    );
    run::run_with_validate(
        &store,
        &spec,
        &NodeRegistry::builtin(),
        &ctx,
        &run_id,
        input,
        validate,
    )
    .expect("run");
    let path = root.join("trajectories").join(format!("{run_id}.jsonl"));
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn verdicts(lines: &[Value]) -> Vec<String> {
    lines
        .iter()
        .filter(|v| v["kind"] == "run.outcome")
        .map(|v| {
            v.pointer("/fields/verdict")
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect()
}

#[test]
fn failed_run_records_failure_with_the_engine_error_verbatim() {
    let root = tempfile::tempdir().unwrap();
    let lines = run_to_trajectory(
        "unsatisfiable.toml",
        serde_json::json!({"branch": "a"}),
        false,
        root.path(),
    );
    assert_eq!(
        verdicts(&lines),
        ["failure"],
        "exactly one outcome: failure"
    );
    let outcome = lines.iter().find(|v| v["kind"] == "run.outcome").unwrap();
    assert!(
        outcome
            .pointer("/fields/detail")
            .and_then(Value::as_str)
            .unwrap()
            .contains("join can never be satisfied"),
        "{outcome}"
    );
}

#[test]
fn successful_run_records_success_and_no_failure_outcome() {
    let root = tempfile::tempdir().unwrap();
    let lines = run_to_trajectory("diamond.toml", serde_json::json!({}), true, root.path());
    assert_eq!(verdicts(&lines), ["success"]);
    assert!(lines.iter().all(|v| v["kind"] != "trajectory.truncated"));
}
