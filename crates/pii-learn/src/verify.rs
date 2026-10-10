//! The verifier (spec 124): re-executes the graph a candidate is about in an
//! isolated shadow session and decides from the shadow run's own numbers.
//! Proposes nothing, promotes nothing, edits no config (stage 125 acts).

use std::collections::BTreeMap;
use std::fs;
use std::io::Read as _;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use pii_core::{Budget, CapSet, Clock as _, Ctx, SystemClock, TraceCtx, TrajectoryBridge};
use pii_graph::node::NodeRegistry;
use pii_graph::spec::GraphSpec;
use pii_graph::{run, toml_load};
use pii_observe::{Event, Level, NoRedaction, Observer, TrajectorySink, names};
use pii_store::Store;

use crate::candidate::{Candidate, Kind};
use crate::config::LearnConfig;
use crate::extract::extract_all;
use crate::scan::{RunSummary, scan};
use crate::verdict::{
    Reason, Verdict, VerdictRecord, VerifyStatus, put_verdict, write_verify_status,
};

const SUBSYSTEM: &str = "learn";
/// Most graph files and the largest one the spec lookup reads.
const MAX_GRAPH_FILES: usize = 1_000;
const MAX_GRAPH_BYTES: u64 = 1024 * 1024;

/// Default most shadow runs one candidate may use (R3).
pub const DEFAULT_MAX_SHADOW_RUNS: u32 = 5;
/// Default wall-clock bound of one shadow run, in ms (R3).
pub const DEFAULT_SHADOW_RUN_MS: u64 = 30_000;

/// Budgets and locations for verification.
#[derive(Debug, Clone)]
pub struct VerifyConfig {
    /// Most shadow runs per candidate.
    pub max_shadow_runs: u32,
    /// Wall-clock bound of each shadow run, ms.
    pub shadow_run_ms: u64,
    /// The input every shadow run gets (the trajectory keeps only its hash).
    pub input: serde_json::Value,
    /// The trajectory root the candidate's evidence paths are relative to.
    pub root: PathBuf,
    /// Where graph specs live (`*.toml`), read-only.
    pub graph_dir: PathBuf,
    /// Parent of the throwaway shadow sessions.
    pub scratch_dir: PathBuf,
    /// Paths the shadow session must never be inside of or contain.
    pub protected: Vec<PathBuf>,
}

impl VerifyConfig {
    /// Defaults over a trajectory `root` and a `graph_dir`.
    #[must_use]
    pub fn new(root: PathBuf, graph_dir: PathBuf) -> Self {
        Self {
            max_shadow_runs: DEFAULT_MAX_SHADOW_RUNS,
            shadow_run_ms: DEFAULT_SHADOW_RUN_MS,
            input: serde_json::Value::Null,
            root,
            graph_dir,
            scratch_dir: std::env::temp_dir(),
            protected: Vec::new(),
        }
    }
}

/// Runs shadow verifications and reports them.
pub struct Verifier {
    cfg: VerifyConfig,
    obs: Arc<dyn Observer>,
}

/// Collects a verdict-in-progress; every early return goes through [`Draft::done`].
struct Draft {
    c: Candidate,
    started: Instant,
    rec: VerdictRecord,
}

impl Draft {
    fn new(c: &Candidate) -> Self {
        Self {
            c: c.clone(),
            started: Instant::now(),
            rec: VerdictRecord {
                candidate: c.id.clone(),
                verdict: Verdict::Inconclusive,
                reason: None,
                detail: String::new(),
                shadow: String::new(),
                shadow_runs: Vec::new(),
                numbers: BTreeMap::new(),
                evidence_used: Vec::new(),
                missing: Vec::new(),
                bound: None,
                elapsed_ms: 0,
            },
        }
    }

    fn inconclusive(mut self, reason: Reason, detail: String) -> VerdictRecord {
        self.rec.verdict = Verdict::Inconclusive;
        self.rec.reason = Some(reason);
        self.rec.detail = detail;
        self.done()
    }

    fn done(mut self) -> VerdictRecord {
        self.rec.elapsed_ms = u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.rec
    }
}

/// Whether `rel` is a plain relative path (no root, no `..`).
fn is_plain_relative(rel: &str) -> bool {
    !rel.is_empty()
        && Path::new(rel)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

/// Finds the graph named `name` among `<graph_dir>/*.toml`, bounded.
fn find_graph(dir: &Path, name: &str) -> Option<GraphSpec> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .ok()?
        .filter_map(|e| Some(e.ok()?.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .collect();
    files.sort();
    for p in files.into_iter().take(MAX_GRAPH_FILES) {
        let Ok(meta) = fs::symlink_metadata(&p) else {
            continue;
        };
        if !meta.is_file() || meta.len() > MAX_GRAPH_BYTES {
            continue;
        }
        let mut text = String::new();
        let ok = fs::File::open(&p)
            .and_then(|f| f.take(MAX_GRAPH_BYTES).read_to_string(&mut text))
            .is_ok();
        if !ok {
            continue;
        }
        if let Ok(spec) = toml_load::load_toml(&text)
            && spec.name == name
        {
            return Some(spec);
        }
    }
    None
}

/// `a` is `b`, inside `b`, or contains `b`.
fn overlaps(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}

/// One shadow run on its own thread, bounded by `limit`. `None` on timeout.
fn run_bounded(
    store: &Arc<Store>,
    spec: &GraphSpec,
    traj: &Path,
    input: &serde_json::Value,
    limit: Duration,
) -> Option<Result<String, String>> {
    let (tx, rx) = mpsc::channel();
    let (store, spec) = (Arc::clone(store), spec.clone());
    let (traj, input) = (traj.to_path_buf(), input.clone());
    std::thread::spawn(move || {
        let out = (|| {
            let run_id = store.next_run_id().map_err(|e| e.to_string())?;
            let sink = Arc::new(TrajectorySink::new(&traj, Arc::new(NoRedaction)));
            // No capabilities: a shadow run cannot touch the workspace.
            let ctx = Ctx::with_observer(
                CapSet::empty(),
                Budget::unlimited(),
                Arc::new(TrajectoryBridge::new(sink, &run_id, None)) as Arc<dyn pii_core::Observer>,
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
                false,
            )
            .map(|_| run_id.to_string())
            .map_err(|e| e.to_string())
        })();
        let _ = tx.send(out);
    });
    rx.recv_timeout(limit).ok()
}

impl Verifier {
    /// A verifier with `cfg`, reporting to `obs`.
    #[must_use]
    pub fn new(cfg: VerifyConfig, obs: Arc<dyn Observer>) -> Self {
        Self { cfg, obs }
    }

    /// Verifies one candidate in an isolated shadow session. Emits
    /// `learn.verification_started` and exactly one verdict event.
    #[must_use]
    pub fn verify(&self, c: &Candidate) -> VerdictRecord {
        self.obs.event(
            &Event::new(SUBSYSTEM, Level::Info, names::LEARN_VERIFICATION_STARTED)
                .field("candidate", c.id.as_str())
                .field("kind", c.kind.as_str()),
        );
        let r = self.decide(Draft::new(c));
        self.report(&r);
        r
    }

    fn report(&self, r: &VerdictRecord) {
        let (name, level) = match r.verdict {
            Verdict::Confirmed => (names::LEARN_CANDIDATE_CONFIRMED, Level::Info),
            Verdict::Refuted => (names::LEARN_CANDIDATE_REFUTED, Level::Info),
            Verdict::Inconclusive => (names::LEARN_CANDIDATE_INCONCLUSIVE, Level::Warn),
        };
        let mut e = Event::new(SUBSYSTEM, level, name)
            .field("candidate", r.candidate.as_str())
            .field("shadow", r.shadow.as_str())
            .field("elapsed_ms", r.elapsed_ms);
        for k in ["shadow_runs", "shadow_failed", "threshold"] {
            if let Some(v) = r.numbers.get(k) {
                e = e.field(k, *v);
            }
        }
        if let Some(reason) = r.reason {
            e = e.field("reason", reason.as_str());
        }
        if let Some(b) = &r.bound {
            e = e.field("bound", b.as_str());
        }
        self.obs.event(&e);
    }

    /// The cited summaries, or the verdict explaining why there are none.
    fn evidence(&self, d: &mut Draft) -> Result<Vec<RunSummary>, (Reason, String)> {
        let paths = d.c.evidence.paths.clone();
        let root = dunce::canonicalize(&self.cfg.root)
            .map_err(|e| (Reason::EvidenceUnreadable, format!("root: {e}")))?;
        let mut missing = Vec::new();
        for p in &paths {
            let ok = is_plain_relative(p)
                && fs::symlink_metadata(root.join(p)).is_ok_and(|m| m.is_file());
            if !ok {
                missing.push(p.clone());
            }
        }
        if !missing.is_empty() {
            let detail = format!(
                "{} of {} cited artefacts are gone (retention?): {}",
                missing.len(),
                paths.len(),
                missing.join(", ")
            );
            d.rec.missing = missing;
            return Err((Reason::EvidenceMissing, detail));
        }
        let (runs, _) = scan(&root, &LearnConfig::default())
            .map_err(|e| (Reason::EvidenceUnreadable, e.to_string()))?;
        let cited: Vec<RunSummary> = runs
            .into_iter()
            .filter(|r| paths.contains(&r.path))
            .collect();
        if cited.len() < paths.len() {
            return Err((
                Reason::EvidenceUnreadable,
                format!(
                    "{} of {} cited artefacts could not be read",
                    paths.len() - cited.len(),
                    paths.len()
                ),
            ));
        }
        d.rec.evidence_used = paths;
        Ok(cited)
    }

    #[allow(clippy::too_many_lines)] // reason: one linear decision procedure; each step is an early return
    fn decide(&self, mut d: Draft) -> VerdictRecord {
        if !matches!(d.c.kind, Kind::RepeatedFailure | Kind::StuckLoop) {
            let detail = format!(
                "{} claims are about limits or cost, which re-running a graph does not test",
                d.c.kind.as_str()
            );
            return d.inconclusive(Reason::NotTestable, detail);
        }
        let cited = match self.evidence(&mut d) {
            Ok(c) => c,
            Err((reason, detail)) => return d.inconclusive(reason, detail),
        };
        let needed =
            d.c.evidence
                .counts
                .get("threshold")
                .copied()
                .unwrap_or(1)
                .max(1);
        if needed > u64::from(self.cfg.max_shadow_runs) {
            d.rec.bound = Some(format!("max_shadow_runs={}", self.cfg.max_shadow_runs));
            let detail = format!(
                "claim needs {needed} shadow runs, over the cap max_shadow_runs={}",
                self.cfg.max_shadow_runs
            );
            return d.inconclusive(Reason::RunCap, detail);
        }
        let Some(graph) = cited.first().map(|r| r.graph.clone()) else {
            return d.inconclusive(Reason::EvidenceUnreadable, String::from("no cited run"));
        };
        let Some(spec) = find_graph(&self.cfg.graph_dir, &graph) else {
            let detail = format!("no graph named {graph} in {}", self.cfg.graph_dir.display());
            return d.inconclusive(Reason::GraphUnavailable, detail);
        };
        let session = match self.session() {
            Ok(s) => s,
            Err(detail) => return d.inconclusive(Reason::Isolation, detail),
        };
        let sid = format!("s-{}", d.c.id.trim_start_matches("c-"));
        d.rec.shadow.clone_from(&sid);
        let traj = session.path().join("traj");
        let Ok(store) = Store::open(&session.path().join("shadow.redb")).map(Arc::new) else {
            return d.inconclusive(
                Reason::Isolation,
                String::from("cannot open the shadow store"),
            );
        };
        let limit = Duration::from_millis(self.cfg.shadow_run_ms);
        for _ in 0..needed {
            let t0 = Instant::now();
            match run_bounded(&store, &spec, &traj, &self.cfg.input, limit) {
                None => {
                    d.rec.bound = Some(format!("shadow_run_ms={}", self.cfg.shadow_run_ms));
                    let detail = format!(
                        "a shadow run exceeded shadow_run_ms={} (elapsed {} ms)",
                        self.cfg.shadow_run_ms,
                        t0.elapsed().as_millis()
                    );
                    return d.inconclusive(Reason::Timeout, detail);
                }
                Some(Err(e)) => {
                    return d.inconclusive(Reason::EngineError, format!("shadow run: {e}"));
                }
                Some(Ok(id)) => d.rec.shadow_runs.push(format!("{sid}/{id}")),
            }
        }
        Self::judge(d, &traj, needed)
    }

    /// A fresh shadow session directory under the scratch dir, or why not.
    fn session(&self) -> Result<tempfile::TempDir, String> {
        let scratch = dunce::canonicalize(&self.cfg.scratch_dir)
            .map_err(|e| format!("scratch {}: {e}", self.cfg.scratch_dir.display()))?;
        let mut guarded: Vec<PathBuf> = self.cfg.protected.clone();
        guarded.extend([self.cfg.root.clone(), self.cfg.graph_dir.clone()]);
        for p in guarded {
            let p = dunce::canonicalize(&p).unwrap_or(p);
            if overlaps(&scratch, &p) {
                return Err(format!(
                    "scratch {} overlaps protected {}",
                    scratch.display(),
                    p.display()
                ));
            }
        }
        tempfile::Builder::new()
            .prefix("pii-shadow-")
            .tempdir_in(&scratch)
            .map_err(|e| format!("cannot create a shadow session: {e}"))
    }

    /// Reads the shadow runs' own trajectories and applies the SAME extractor
    /// that proposed the claim: the claim holds iff it comes out again.
    fn judge(mut d: Draft, traj: &Path, needed: u64) -> VerdictRecord {
        let cfg = LearnConfig {
            min_runs: u32::try_from(needed).unwrap_or(u32::MAX),
            ..LearnConfig::default()
        };
        let Ok((runs, _)) = scan(traj, &cfg) else {
            return d.inconclusive(
                Reason::EvidenceUnreadable,
                String::from("shadow trajectories unreadable"),
            );
        };
        let failed = runs.iter().filter(|r| r.verdict == "failure").count() as u64;
        d.rec.numbers = BTreeMap::from([
            ("shadow_runs".to_owned(), runs.len() as u64),
            ("shadow_failed".to_owned(), failed),
            ("threshold".to_owned(), needed),
        ]);
        let reproduced = extract_all(&runs, &cfg)
            .candidates
            .iter()
            .any(|x| x.kind == d.c.kind && x.dedupe_key == d.c.dedupe_key);
        d.rec.verdict = if reproduced {
            Verdict::Confirmed
        } else {
            Verdict::Refuted
        };
        let mut seen: BTreeMap<String, u64> = BTreeMap::new();
        for (node, class) in runs.iter().flat_map(|r| &r.failures) {
            *seen.entry(format!("{node}:{class}")).or_default() += 1;
        }
        d.rec.detail = format!(
            "{} of {} shadow runs failed; claim {} (needs {needed} matching runs); observed failures {seen:?}",
            failed,
            runs.len(),
            if reproduced {
                "reproduced"
            } else {
                "did not reproduce"
            }
        );
        d.done()
    }

    /// Verifies every candidate and records each verdict under `dir`, then the
    /// pass status. Writes nothing else.
    ///
    /// # Errors
    /// A message when a verdict or the status cannot be written.
    pub fn verify_all(&self, dir: &Path, cs: &[Candidate]) -> Result<Vec<VerdictRecord>, String> {
        let mut out = Vec::new();
        let mut verdicts: BTreeMap<String, u64> = Verdict::ALL
            .iter()
            .map(|v| (v.as_str().to_owned(), 0))
            .collect();
        for c in cs {
            let r = self.verify(c);
            put_verdict(dir, &r).map_err(|e| e.to_string())?;
            *verdicts.entry(r.verdict.as_str().to_owned()).or_default() += 1;
            out.push(r);
        }
        let at_secs = SystemClock.now_ms() / 1_000;
        let status = VerifyStatus {
            at_secs,
            candidates: out.len() as u64,
            verdicts,
        };
        write_verify_status(dir, &status).map_err(|e| e.to_string())?;
        Ok(out)
    }
}
