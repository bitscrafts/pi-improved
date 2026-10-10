//! Spec 124: the verifier. Fixtures are runs of the REAL engine (real
//! scheduler, bridge and trajectory sink); nothing here is hand-written JSON
//! except the deliberately fabricated candidates.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::many_single_char_names
)]

use std::collections::BTreeMap;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use pii_core::{Budget, CapSet, Ctx, Observer, TraceCtx, TrajectoryBridge};
use pii_graph::node::NodeRegistry;
use pii_graph::run;
use pii_graph::toml_load::load_toml;
use pii_learn::{
    Candidate, Evidence, Kind, LearnConfig, Reason, Verdict, Verifier, VerifyConfig, extract_all,
    get_verdict, read_verify_status, scan,
};
use pii_observe::{NoRedaction, Recorder, TrajectorySink, names};
use pii_store::Store;
use serde_json::json;
use tempfile::TempDir;

fn ws() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .to_path_buf()
}

/// Runs `examples/<name>.toml` once through the real engine, trajectory at `root`.
fn real_run(name: &str, root: &Path, db: &Path) {
    let src = fs::read_to_string(ws().join("examples").join(format!("{name}.toml"))).unwrap();
    let spec = load_toml(&src).unwrap();
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
        json!({"branch": "a"}),
        false,
    )
    .unwrap();
}

struct Fixture {
    tmp: TempDir,
    root: PathBuf,
    graphs: PathBuf,
    scratch: PathBuf,
}

/// Three real runs of each named example, a graph dir holding those specs, and
/// an empty scratch dir.
fn fixture(names_: &[&str]) -> Fixture {
    let t = TempDir::new().unwrap();
    let root = t.path().join("traj");
    let graphs = t.path().join("graphs");
    let scratch = t.path().join("scratch");
    for d in [&root, &graphs, &scratch] {
        fs::create_dir_all(d).unwrap();
    }
    for n in names_ {
        fs::copy(
            ws().join("examples").join(format!("{n}.toml")),
            graphs.join(format!("{n}.toml")),
        )
        .unwrap();
        for _ in 0..3 {
            real_run(n, &root, &t.path().join("db.redb"));
        }
    }
    Fixture {
        tmp: t,
        root,
        graphs,
        scratch,
    }
}

fn verifier(f: &Fixture, edit: impl FnOnce(&mut VerifyConfig)) -> (Verifier, Arc<Recorder>) {
    let mut cfg = VerifyConfig::new(f.root.clone(), f.graphs.clone());
    cfg.scratch_dir.clone_from(&f.scratch);
    edit(&mut cfg);
    let rec = Arc::new(Recorder::new());
    (Verifier::new(cfg, rec.clone()), rec)
}

/// The repeated-failure candidate the real extractor emits for `f`'s failing runs.
fn real_candidate(f: &Fixture) -> Candidate {
    let cfg = LearnConfig::default();
    let (runs, _) = scan(&f.root, &cfg).unwrap();
    extract_all(&runs, &cfg)
        .candidates
        .into_iter()
        .find(|c| c.kind == Kind::RepeatedFailure)
        .expect("a repeated_failure candidate")
}

/// A claim written by hand that is NOT true of its evidence: `diamond` always
/// succeeds, yet this says it failed at `join` in three runs.
fn fabricated(f: &Fixture) -> Candidate {
    let (runs, _) = scan(&f.root, &LearnConfig::default()).unwrap();
    let ds: Vec<_> = runs.iter().filter(|r| r.graph == "diamond").collect();
    assert_eq!(ds.len(), 3);
    Candidate::new(
        Kind::RepeatedFailure,
        "node join in graph diamond failed with class other in 3 runs (3 failures, threshold 3)"
            .into(),
        "repeated_failure:diamond:join:other".into(),
        Evidence {
            runs: ds.iter().map(|r| r.run.clone()).collect(),
            paths: ds.iter().map(|r| r.path.clone()).collect(),
            counts: BTreeMap::from([
                ("runs".into(), 3),
                ("failures".into(), 3),
                ("threshold".into(), 3),
            ]),
        },
    )
    .unwrap()
}

/// File name, size and content hash of everything under `dir`, sorted.
fn snapshot(dir: &Path) -> Vec<(String, u64, String)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else {
                let b = fs::read(&p).unwrap();
                out.push((
                    p.strip_prefix(dir).unwrap().display().to_string(),
                    b.len() as u64,
                    blake3::hash(&b).to_hex().to_string(),
                ));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn a_real_failure_claim_is_confirmed_with_the_shadow_runs_own_numbers() {
    let f = fixture(&["unsatisfiable"]);
    let c = real_candidate(&f);
    let (v, rec) = verifier(&f, |_| {});
    let r = v.verify(&c);
    assert_eq!(r.verdict, Verdict::Confirmed, "{r:?}");
    assert_eq!(r.reason, None);
    assert_eq!(r.numbers["shadow_runs"], 3);
    assert_eq!(r.numbers["shadow_failed"], 3);
    assert_eq!(r.shadow_runs.len(), 3);
    assert!(r.shadow.starts_with("s-"), "{}", r.shadow);
    assert!(r.shadow_runs.iter().all(|id| id.starts_with(&r.shadow)));
    assert_eq!(r.evidence_used, c.evidence.paths);
    assert_eq!(rec.events_named(names::LEARN_VERIFICATION_STARTED).len(), 1);
    assert_eq!(rec.events_named(names::LEARN_CANDIDATE_CONFIRMED).len(), 1);
}

#[test]
fn a_fabricated_claim_is_refuted_and_refutation_is_reachable() {
    let f = fixture(&["unsatisfiable", "diamond"]);
    let c = fabricated(&f);
    let (v, rec) = verifier(&f, |_| {});
    let r = v.verify(&c);
    assert_eq!(r.verdict, Verdict::Refuted, "{r:?}");
    assert_eq!(r.numbers["shadow_runs"], 3);
    assert_eq!(
        r.numbers["shadow_failed"], 0,
        "diamond succeeds in the shadow"
    );
    assert!(!r.shadow_runs.is_empty());
    assert_eq!(rec.events_named(names::LEARN_CANDIDATE_REFUTED).len(), 1);
    assert!(
        rec.events_named(names::LEARN_CANDIDATE_CONFIRMED)
            .is_empty()
    );
}

#[test]
fn missing_evidence_is_inconclusive_naming_the_paths_never_refuted() {
    let f = fixture(&["unsatisfiable"]);
    let mut c = real_candidate(&f);
    c.evidence.paths[0] = "trajectories/rGONE.jsonl".into();
    let (v, rec) = verifier(&f, |_| {});
    let r = v.verify(&c);
    assert_eq!(r.verdict, Verdict::Inconclusive, "{r:?}");
    assert_eq!(r.reason, Some(Reason::EvidenceMissing));
    assert_eq!(r.missing, vec!["trajectories/rGONE.jsonl".to_owned()]);
    assert!(r.detail.contains("rGONE"), "{}", r.detail);
    assert!(r.shadow_runs.is_empty(), "nothing ran");
    let e = rec.events_named(names::LEARN_CANDIDATE_INCONCLUSIVE);
    assert_eq!(e.len(), 1);
}

#[test]
fn the_run_cap_is_enforced_and_named() {
    let f = fixture(&["unsatisfiable"]);
    let c = real_candidate(&f);
    let (v, _) = verifier(&f, |cfg| cfg.max_shadow_runs = 2);
    let r = v.verify(&c);
    assert_eq!(r.verdict, Verdict::Inconclusive, "{r:?}");
    assert_eq!(r.reason, Some(Reason::RunCap));
    assert_eq!(r.bound.as_deref(), Some("max_shadow_runs=2"));
    assert!(r.shadow_runs.is_empty(), "no run over the cap");
}

#[test]
fn the_wall_clock_bound_is_enforced_and_reports_elapsed() {
    let f = fixture(&["unsatisfiable"]);
    let c = real_candidate(&f);
    let (v, _) = verifier(&f, |cfg| cfg.shadow_run_ms = 1);
    let t0 = std::time::Instant::now();
    let r = v.verify(&c);
    assert!(t0.elapsed().as_secs() < 10, "never a hang");
    assert_eq!(r.verdict, Verdict::Inconclusive, "{r:?}");
    assert_eq!(r.reason, Some(Reason::Timeout));
    assert_eq!(r.bound.as_deref(), Some("shadow_run_ms=1"));
    assert!(r.detail.contains("elapsed"), "{}", r.detail);
}

#[test]
fn limit_and_cost_claims_are_not_testable_by_rerun() {
    let f = fixture(&["unsatisfiable"]);
    let mut c = real_candidate(&f);
    c.kind = Kind::LimitHit;
    let (v, _) = verifier(&f, |_| {});
    let r = v.verify(&c);
    assert_eq!(r.reason, Some(Reason::NotTestable), "{r:?}");
}

#[test]
fn an_unknown_graph_is_inconclusive_not_refuted() {
    let f = fixture(&["unsatisfiable"]);
    let c = real_candidate(&f);
    let empty = f.scratch.join("no-graphs");
    fs::create_dir_all(&empty).unwrap();
    let (v, _) = verifier(&f, |cfg| cfg.graph_dir = empty);
    let r = v.verify(&c);
    assert_eq!(r.reason, Some(Reason::GraphUnavailable), "{r:?}");
}

#[test]
fn the_verifier_leaves_production_state_byte_identical() {
    let f = fixture(&["unsatisfiable", "diamond"]);
    // A production store with content, and a configured workspace with a file.
    let prod_dir = f.tmp.path().join("prod");
    let workspace = f.tmp.path().join("workspace");
    fs::create_dir_all(&prod_dir).unwrap();
    fs::create_dir_all(&workspace).unwrap();
    fs::write(workspace.join("notes.txt"), b"workspace content").unwrap();
    let prod_db = prod_dir.join("store.redb");
    real_run("diamond", &f.tmp.path().join("prod-traj"), &prod_db);
    // Opening a redb file rewrites it, so the key count is read from a COPY.
    let info = |p: &Path| {
        let copy = f.tmp.path().join("measure-copy.redb");
        fs::copy(p, &copy).unwrap();
        let i = Store::open(&copy).unwrap().info().unwrap();
        fs::remove_file(&copy).unwrap();
        (
            i.tables.values().sum::<u64>(),
            fs::metadata(p).unwrap().len(),
        )
    };
    let before = (
        info(&prod_db),
        snapshot(&prod_dir),
        snapshot(&workspace),
        snapshot(&f.graphs),
        snapshot(&f.root),
    );
    let _ = writeln!(
        std::io::stderr(),
        "BEFORE production store: keys={} bytes={}",
        before.0.0,
        before.0.1
    );

    let learn_dir = f.tmp.path().join("learn-dir");
    let cs = vec![real_candidate(&f), fabricated(&f)];
    let (v, _) = verifier(&f, |cfg| {
        cfg.protected = vec![prod_dir.clone(), workspace.clone()];
    });
    let rs = v.verify_all(&learn_dir, &cs).unwrap();
    assert_eq!(rs.len(), 2);
    assert!(rs.iter().all(|r| !r.shadow_runs.is_empty()), "{rs:?}");

    let after = (
        info(&prod_db),
        snapshot(&prod_dir),
        snapshot(&workspace),
        snapshot(&f.graphs),
        snapshot(&f.root),
    );
    let _ = writeln!(
        std::io::stderr(),
        "AFTER  production store: keys={} bytes={}",
        after.0.0,
        after.0.1
    );
    assert_eq!(before, after, "production state changed");
    // The shadow sessions are gone from scratch; only verdicts were written.
    assert!(snapshot(&f.scratch).is_empty(), "shadow left files behind");
    let written: Vec<_> = snapshot(&learn_dir).into_iter().map(|x| x.0).collect();
    assert!(
        written
            .iter()
            .all(|p| p.starts_with("learn/verdicts/") || p == "learn/verify_status.json"),
        "{written:?}"
    );
}

#[test]
fn a_scratch_dir_inside_protected_state_is_refused_not_run_best_effort() {
    let f = fixture(&["unsatisfiable"]);
    let c = real_candidate(&f);
    let prod = f.tmp.path().join("prod");
    fs::create_dir_all(&prod).unwrap();
    fs::write(prod.join("store.redb"), b"x").unwrap();
    let before = snapshot(&prod);
    let (v, _) = verifier(&f, |cfg| {
        cfg.scratch_dir = prod.clone();
        cfg.protected = vec![prod.clone()];
    });
    let r = v.verify(&c);
    assert_eq!(r.reason, Some(Reason::Isolation), "{r:?}");
    assert!(r.shadow_runs.is_empty());
    assert_eq!(
        before,
        snapshot(&prod),
        "nothing written to protected state"
    );
}

#[test]
fn every_verdict_of_a_batch_is_recorded_per_candidate() {
    let f = fixture(&["unsatisfiable", "diamond"]);
    let (real, fake) = (real_candidate(&f), fabricated(&f));
    let learn_dir = f.tmp.path().join("learn-dir");
    let (v, _) = verifier(&f, |_| {});
    let rs = v
        .verify_all(&learn_dir, &[real.clone(), fake.clone()])
        .unwrap();
    assert_eq!(rs.len(), 2);
    let got_real = get_verdict(&learn_dir, &real.id).expect("recorded");
    let got_fake = get_verdict(&learn_dir, &fake.id).expect("recorded");
    assert_eq!(got_real.verdict, Verdict::Confirmed);
    assert_eq!(got_fake.verdict, Verdict::Refuted, "not averaged away");
    let st = read_verify_status(&learn_dir).expect("status");
    assert_eq!(st.candidates, 2);
    assert_eq!(
        (
            st.verdicts["confirmed"],
            st.verdicts["refuted"],
            st.verdicts["inconclusive"]
        ),
        (1, 1, 0)
    );
}
