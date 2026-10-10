//! Spec 123: extraction over trajectories the REAL engine wrote. The runs go
//! through the real scheduler, `TrajectoryBridge` and `TrajectorySink` (no
//! hand-written JSON), then `pii-learn` reads them back.
//!
//! `PII_LEARN_E2E_ROOT=<dir>` keeps the trajectories there (for the report's
//! "candidate from a real trajectory"); unset, they live in a tempdir.

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
use pii_learn::{Kind, LearnConfig, extract_all, scan};
use pii_observe::{NoRedaction, TrajectorySink};
use pii_store::Store;

/// One run of `examples/unsatisfiable.toml` (it fails: a join that can never be
/// satisfied) with the real sink at `root`.
fn fail_once(root: &Path, db: &Path) {
    let ws = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap();
    let spec = load_toml(&std::fs::read_to_string(ws.join("examples/unsatisfiable.toml")).unwrap())
        .unwrap();
    let store = Store::open(db).unwrap();
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
        serde_json::json!({"branch": "a"}),
        false,
    )
    .unwrap();
}

#[test]
fn three_real_failing_runs_become_cited_candidates() {
    let kept = pii_core::env_var("PII_LEARN_E2E_ROOT").map(PathBuf::from);
    let tmp = tempfile::tempdir().unwrap();
    let root = kept.unwrap_or_else(|| tmp.path().join("traj"));
    std::fs::create_dir_all(&root).unwrap();
    for _ in 0..3 {
        fail_once(&root, &tmp.path().join("db.redb"));
    }
    let cfg = LearnConfig::default();
    let (runs, stats) = scan(&root, &cfg).unwrap();
    assert!(runs.len() >= 3, "{stats:?}");
    let ex = extract_all(&runs, &cfg);
    let rf: Vec<_> = ex
        .candidates
        .iter()
        .filter(|c| c.kind == Kind::RepeatedFailure)
        .collect();
    assert_eq!(rf.len(), 1, "{:?}", ex.candidates);
    assert!(
        rf[0].evidence.paths.iter().all(|p| root.join(p).is_file()),
        "cited artefacts exist"
    );
    assert!(rf[0].evidence.counts["runs"] >= 3);
}
