//! The trajectory sink: an append-only, redacted, bounded JSONL record of a
//! run, one file per run at `<root>/trajectories/<run_id>.jsonl`.
//!
//! The sink is an [`Observer`]; it only records events carrying a run id.
//! Redaction is applied here, to a copy of every event, so the original
//! event (and every other observer's view of it) is untouched. The sink never
//! truncates or rewrites a file; the only thing it deletes is the OLDEST
//! finished trajectories when an aggregate [`Retention`] bound is exceeded
//! (spec 115, never a run still open, always reported as
//! [`names::TRAJECTORY_PRUNED`]). A write failure is reported as
//! a [`names::TRAJECTORY_WRITE_FAILED`] event to the reporter observer — it
//! never panics and never aborts the run.

use std::collections::{BTreeMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;

use crate::retention::{Index, Retention};
use crate::{Event, Level, NullObserver, Observer, Redaction, RedactionPolicy, RunId, names};

/// Per-run bounds on a trajectory file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrajectoryCaps {
    /// Maximum events recorded per run.
    pub max_events: usize,
    /// Maximum bytes recorded per run.
    pub max_bytes: u64,
}

impl Default for TrajectoryCaps {
    fn default() -> Self {
        Self {
            max_events: 10_000,
            max_bytes: 16 * 1024 * 1024,
        }
    }
}

#[derive(Default)]
struct RunFile {
    file: Option<File>,
    events: usize,
    bytes: u64,
    truncated: bool,
    outcome_written: bool,
    failure_reported: bool,
}

/// Writes correlated events to per-run JSONL trajectories.
// reason: pii-observe is the dependency floor (serde only); it cannot take
// parking_lot, and a poisoned lock just drops a record.
#[allow(clippy::disallowed_types)]
pub struct TrajectorySink {
    root: PathBuf,
    policy: Arc<dyn RedactionPolicy>,
    caps: TrajectoryCaps,
    reporter: Arc<dyn Observer>,
    runs: std::sync::Mutex<BTreeMap<String, RunFile>>,
    retention: Retention,
    index: std::sync::Mutex<Index>,
}

impl std::fmt::Debug for TrajectorySink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrajectorySink")
            .field("root", &self.root)
            .field("caps", &self.caps)
            .finish_non_exhaustive()
    }
}

impl TrajectorySink {
    /// A sink writing under `root` with the given redaction policy and the
    /// default caps. Failures are reported to a [`NullObserver`] until
    /// [`Self::with_reporter`] is used.
    #[must_use]
    #[allow(clippy::disallowed_types)] // reason: see the struct
    pub fn new(root: impl Into<PathBuf>, policy: Arc<dyn RedactionPolicy>) -> Self {
        Self {
            root: root.into(),
            policy,
            caps: TrajectoryCaps::default(),
            reporter: Arc::new(NullObserver),
            runs: std::sync::Mutex::default(),
            retention: Retention::default(),
            index: std::sync::Mutex::default(),
        }
    }

    /// Bounds the whole trajectory directory (spec 115). Default: unbounded.
    #[must_use]
    pub fn with_retention(mut self, retention: Retention) -> Self {
        self.retention = retention;
        self
    }

    /// Overrides the per-run caps.
    #[must_use]
    pub fn with_caps(mut self, caps: TrajectoryCaps) -> Self {
        self.caps = caps;
        self
    }

    /// Where write failures and truncations are reported.
    #[must_use]
    pub fn with_reporter(mut self, reporter: Arc<dyn Observer>) -> Self {
        self.reporter = reporter;
        self
    }

    /// The trajectory file for `run`.
    #[must_use]
    pub fn path_for(&self, run: &RunId) -> PathBuf {
        self.root
            .join("trajectories")
            .join(format!("{}.jsonl", sanitize(&run.0)))
    }

    fn report_failure(&self, run: &RunId, why: &str) {
        self.reporter.event(
            &Event::new("observe", Level::Error, names::TRAJECTORY_WRITE_FAILED)
                .field("run", run.0.as_str())
                .field("error", why)
                .in_run(run.clone()),
        );
    }

    fn open(path: &Path) -> std::io::Result<File> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        OpenOptions::new().create(true).append(true).open(path)
    }

    fn record(&self, e: &Event) {
        let Some(run) = e.run.clone() else { return };
        let line = match serde_json::to_value(e) {
            Ok(mut v) => {
                if let Some(Value::Object(f)) = v.get_mut("fields") {
                    let taken = std::mem::take(f);
                    *f = redact_object(self.policy.as_ref(), "", taken);
                }
                format!("{v}\n")
            }
            Err(err) => {
                self.report_failure(&run, &err.to_string());
                return;
            }
        };
        let is_outcome = e.kind == names::RUN_OUTCOME;
        let Ok(mut runs) = self.runs.lock() else {
            return;
        };
        let st = runs.entry(run.0.clone()).or_default();
        if st.truncated && (!is_outcome || st.outcome_written) {
            return;
        }
        let path = self.path_for(&run);
        if st.file.is_none() {
            if let Ok(mut idx) = self.index.lock() {
                idx.ensure_scanned(&self.root.join("trajectories"));
            }
            match Self::open(&path) {
                Ok(f) => {
                    st.bytes = f.metadata().map_or(0, |m| m.len());
                    st.file = Some(f);
                    if let (Ok(mut idx), Some(name)) =
                        (self.index.lock(), path.file_name().and_then(|f| f.to_str()))
                    {
                        idx.created(name, st.bytes);
                    }
                }
                Err(err) => {
                    if !st.failure_reported {
                        st.failure_reported = true;
                        self.report_failure(&run, &err.to_string());
                    }
                    return;
                }
            }
        }
        let over =
            st.events >= self.caps.max_events || st.bytes + line.len() as u64 > self.caps.max_bytes;
        // The outcome is one line and the record that matters most: it is
        // admitted past the cap once, so a capped run still ends verbatim.
        if over && !is_outcome {
            st.truncated = true;
            self.mark_truncated(st, &run);
            return;
        }
        if over {
            st.truncated = true;
        }
        if is_outcome {
            st.outcome_written = true;
        }
        let Some(file) = st.file.as_mut() else { return };
        match file.write_all(line.as_bytes()) {
            Ok(()) => {
                st.events += 1;
                st.bytes += line.len() as u64;
                // A finished run's handle is released so its file can be pruned.
                if is_outcome {
                    st.file = None;
                }
                self.account(&runs, &path, line.len() as u64);
            }
            Err(err) => {
                if !st.failure_reported {
                    st.failure_reported = true;
                    self.report_failure(&run, &err.to_string());
                }
            }
        }
    }

    /// Books `n` appended bytes against the retention index and enforces the
    /// policy. Runs with an open handle are in flight and are never pruned.
    fn account(&self, runs: &BTreeMap<String, RunFile>, path: &Path, n: u64) {
        let Some(name) = path.file_name().and_then(|f| f.to_str()) else {
            return;
        };
        let Ok(mut idx) = self.index.lock() else {
            return;
        };
        idx.wrote(name, n);
        let mut open: HashSet<String> = runs
            .iter()
            .filter(|(_, st)| st.file.is_some())
            .map(|(id, _)| format!("{}.jsonl", sanitize(id)))
            .collect();
        // The run being written right now is current even if it just finished.
        open.insert(name.to_owned());
        idx.enforce(
            &self.root.join("trajectories"),
            &self.retention,
            &open,
            self.reporter.as_ref(),
        );
    }

    fn mark_truncated(&self, st: &mut RunFile, run: &RunId) {
        let marker = Event::new("observe", Level::Warn, names::TRAJECTORY_TRUNCATED)
            .field("max_events", self.caps.max_events as u64)
            .field("max_bytes", self.caps.max_bytes)
            .in_run(run.clone());
        if let (Some(file), Ok(line)) = (st.file.as_mut(), serde_json::to_string(&marker)) {
            // reason: the marker is best-effort; the report below still fires.
            let _ = writeln!(file, "{line}");
        }
        self.reporter.event(&marker);
    }
}

impl Observer for TrajectorySink {
    fn event(&self, e: &Event) {
        self.record(e);
    }

    fn flush(&self) {
        if let Ok(mut runs) = self.runs.lock() {
            for st in runs.values_mut() {
                if let Some(f) = st.file.as_mut() {
                    // reason: nowhere to report a failed flush; appends are unbuffered.
                    let _ = f.flush();
                }
            }
        }
    }
}

/// A run id as a safe file stem: anything outside `[A-Za-z0-9._-]` becomes `_`.
fn sanitize(id: &str) -> String {
    let s: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() { String::from("_") } else { s }
}

fn join(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_owned()
    } else {
        format!("{path}.{key}")
    }
}

fn redact_object(
    policy: &dyn RedactionPolicy,
    path: &str,
    obj: serde_json::Map<String, Value>,
) -> serde_json::Map<String, Value> {
    obj.into_iter()
        .filter_map(|(k, v)| redact_value(policy, &join(path, &k), v).map(|v| (k, v)))
        .collect()
}

fn redact_value(policy: &dyn RedactionPolicy, path: &str, v: Value) -> Option<Value> {
    match v {
        Value::Object(o) => Some(Value::Object(redact_object(policy, path, o))),
        Value::Array(a) => Some(Value::Array(
            a.into_iter()
                .filter_map(|x| redact_value(policy, path, x))
                .collect(),
        )),
        Value::Null => Some(Value::Null),
        Value::String(s) => apply(policy.redact(path, &s), Value::String(s)),
        other => {
            let text = other.to_string();
            apply(policy.redact(path, &text), other)
        }
    }
}

fn apply(r: Redaction, original: Value) -> Option<Value> {
    match r {
        Redaction::Keep => Some(original),
        Redaction::Drop => None,
        Redaction::Replace(t) => Some(Value::String(t)),
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::*;
    use crate::{NoRedaction, Outcome, Recorder, Verdict};

    fn ev(kind: &'static str) -> Event {
        Event::new("graph", Level::Info, kind).in_run(RunId("r1".into()))
    }

    fn lines(sink: &TrajectorySink) -> Vec<Value> {
        let p = sink.path_for(&RunId("r1".into()));
        std::fs::read_to_string(p)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    struct Policy;
    impl RedactionPolicy for Policy {
        fn redact(&self, field: &str, value: &str) -> Redaction {
            match field {
                "token" => Redaction::Drop,
                "email" => Redaction::Replace("<email>".into()),
                "nested.secret" => Redaction::Replace(format!("<{}>", value.len())),
                _ => Redaction::Keep,
            }
        }
    }

    #[test]
    fn writes_one_json_line_per_event_under_trajectories() {
        let d = tempfile::tempdir().unwrap();
        let sink = TrajectorySink::new(d.path(), Arc::new(NoRedaction));
        sink.event(&ev("run.journal").field("a", 1u64));
        sink.event(&ev("run.journal").field("a", 2u64));
        assert!(d.path().join("trajectories/r1.jsonl").exists());
        let l = lines(&sink);
        assert_eq!(l.len(), 2);
        assert_eq!(l[1].pointer("/fields/a"), Some(&Value::from(2)));
        assert_eq!(l[0].pointer("/run"), Some(&Value::from("r1")));
    }

    #[test]
    fn redaction_matrix_keep_drop_replace_at_the_sink_only() {
        let d = tempfile::tempdir().unwrap();
        let sink = TrajectorySink::new(d.path(), Arc::new(Policy));
        let e = ev("run.journal")
            .field("keep", "visible")
            .field("token", "sk-abc")
            .field("email", "a@b.c")
            .field("nested", serde_json::json!({"secret": "12345", "ok": 1}));
        let before = e.clone();
        sink.event(&e);
        assert_eq!(e, before, "the original event is unaffected");
        let l = lines(&sink);
        assert_eq!(l.len(), 1, "redaction does not drop the record itself");
        let f = l[0].get("fields").unwrap();
        assert_eq!(f.get("keep"), Some(&Value::from("visible")));
        assert_eq!(f.get("token"), None, "Drop removes the field");
        assert_eq!(f.get("email"), Some(&Value::from("<email>")));
        assert_eq!(f.pointer("/nested/secret"), Some(&Value::from("<5>")));
        assert_eq!(f.pointer("/nested/ok"), Some(&Value::from(1)));
        assert!(
            !std::fs::read_to_string(sink.path_for(&RunId("r1".into())))
                .unwrap()
                .contains("sk-abc")
        );
    }

    #[test]
    fn outcome_detail_is_redacted_too() {
        struct DropDetail;
        impl RedactionPolicy for DropDetail {
            fn redact(&self, field: &str, _: &str) -> Redaction {
                if field == "detail" {
                    Redaction::Replace("<redacted>".into())
                } else {
                    Redaction::Keep
                }
            }
        }
        let d = tempfile::tempdir().unwrap();
        let sink = TrajectorySink::new(d.path(), Arc::new(DropDetail));
        let o = Outcome {
            verdict: Verdict::Failure,
            reason: "terminal".into(),
            detail: Value::from("secret failure text"),
        }
        .into_event("graph")
        .in_run(RunId("r1".into()));
        sink.event(&o);
        let l = lines(&sink);
        assert_eq!(
            l[0].pointer("/fields/detail"),
            Some(&Value::from("<redacted>"))
        );
        assert_eq!(
            l[0].pointer("/fields/verdict"),
            Some(&Value::from("failure"))
        );
    }

    #[test]
    fn a_failed_run_records_failure_never_success() {
        let d = tempfile::tempdir().unwrap();
        let sink = TrajectorySink::new(d.path(), Arc::new(NoRedaction));
        sink.event(
            &Outcome {
                verdict: Verdict::Failure,
                reason: "terminal".into(),
                detail: Value::from("exit 1"),
            }
            .into_event("graph")
            .in_run(RunId("r1".into())),
        );
        let l = lines(&sink);
        let outcomes: Vec<_> = l.iter().filter(|v| v["kind"] == "run.outcome").collect();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(
            outcomes[0].pointer("/fields/verdict"),
            Some(&Value::from("failure"))
        );
        assert!(
            !l.iter()
                .any(|v| v.pointer("/fields/verdict") == Some(&Value::from("success")))
        );
    }

    #[test]
    fn a_successful_run_emits_no_failure_outcome() {
        let d = tempfile::tempdir().unwrap();
        let sink = TrajectorySink::new(d.path(), Arc::new(NoRedaction));
        sink.event(
            &Outcome {
                verdict: Verdict::Success,
                reason: "terminal".into(),
                detail: Value::from("done"),
            }
            .into_event("graph")
            .in_run(RunId("r1".into())),
        );
        assert!(
            !lines(&sink)
                .iter()
                .any(|v| v.pointer("/fields/verdict") == Some(&Value::from("failure")))
        );
    }

    #[test]
    fn cap_truncates_with_one_marker_and_reports_it() {
        let d = tempfile::tempdir().unwrap();
        let rep = Arc::new(Recorder::new());
        let sink = TrajectorySink::new(d.path(), Arc::new(NoRedaction))
            .with_caps(TrajectoryCaps {
                max_events: 3,
                max_bytes: 1 << 20,
            })
            .with_reporter(rep.clone());
        for _ in 0..10 {
            sink.event(&ev("run.journal"));
        }
        let l = lines(&sink);
        assert_eq!(l.len(), 4, "3 events + 1 marker");
        assert_eq!(l[3]["kind"], "trajectory.truncated");
        assert_eq!(rep.events_named(names::TRAJECTORY_TRUNCATED).len(), 1);
    }

    #[test]
    fn byte_cap_truncates() {
        let d = tempfile::tempdir().unwrap();
        let sink = TrajectorySink::new(d.path(), Arc::new(NoRedaction)).with_caps(TrajectoryCaps {
            max_events: 1000,
            max_bytes: 400,
        });
        for _ in 0..20 {
            sink.event(&ev("run.journal").field("pad", "x".repeat(50)));
        }
        let l = lines(&sink);
        assert!(l.len() < 20);
        assert_eq!(l.last().unwrap()["kind"], "trajectory.truncated");
    }

    #[test]
    fn outcome_is_still_recorded_after_truncation() {
        let d = tempfile::tempdir().unwrap();
        let sink = TrajectorySink::new(d.path(), Arc::new(NoRedaction)).with_caps(TrajectoryCaps {
            max_events: 1,
            max_bytes: 1 << 20,
        });
        sink.event(&ev("run.journal"));
        sink.event(&ev("run.journal"));
        sink.event(
            &Outcome {
                verdict: Verdict::Failure,
                reason: "t".into(),
                detail: Value::Null,
            }
            .into_event("graph")
            .in_run(RunId("r1".into())),
        );
        let l = lines(&sink);
        assert_eq!(l.last().unwrap()["kind"], "run.outcome");
    }

    #[test]
    fn short_run_emits_no_truncation() {
        let d = tempfile::tempdir().unwrap();
        let rep = Arc::new(Recorder::new());
        let sink = TrajectorySink::new(d.path(), Arc::new(NoRedaction)).with_reporter(rep.clone());
        for _ in 0..5 {
            sink.event(&ev("run.journal"));
        }
        assert!(
            !lines(&sink)
                .iter()
                .any(|v| v["kind"] == "trajectory.truncated")
        );
        assert!(rep.events_named(names::TRAJECTORY_TRUNCATED).is_empty());
    }

    #[test]
    fn events_without_a_run_write_nothing() {
        let d = tempfile::tempdir().unwrap();
        let sink = TrajectorySink::new(d.path(), Arc::new(NoRedaction));
        sink.event(&Event::new("graph", Level::Info, "run.journal"));
        assert!(!d.path().join("trajectories").exists());
    }

    #[test]
    fn appends_never_rewrites_an_existing_file() {
        let d = tempfile::tempdir().unwrap();
        let s1 = TrajectorySink::new(d.path(), Arc::new(NoRedaction));
        s1.event(&ev("run.journal").field("n", 1u64));
        let s2 = TrajectorySink::new(d.path(), Arc::new(NoRedaction));
        s2.event(&ev("run.journal").field("n", 2u64));
        let l = lines(&s2);
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].pointer("/fields/n"), Some(&Value::from(1)));
    }

    #[test]
    fn write_failure_is_reported_as_an_event_and_does_not_panic() {
        let d = tempfile::tempdir().unwrap();
        // `trajectories` exists as a FILE, so create_dir_all fails.
        std::fs::write(d.path().join("trajectories"), b"x").unwrap();
        let rep = Arc::new(Recorder::new());
        let sink = TrajectorySink::new(d.path(), Arc::new(NoRedaction)).with_reporter(rep.clone());
        sink.event(&ev("run.journal"));
        sink.event(&ev("run.journal"));
        assert_eq!(rep.events_named(names::TRAJECTORY_WRITE_FAILED).len(), 1);
        assert_eq!(std::fs::read(d.path().join("trajectories")).unwrap(), b"x");
    }

    #[test]
    fn run_id_cannot_escape_the_trajectory_dir() {
        let d = tempfile::tempdir().unwrap();
        let sink = TrajectorySink::new(d.path(), Arc::new(NoRedaction));
        let p = sink.path_for(&RunId("../../etc/x".into()));
        assert_eq!(p.parent().unwrap(), d.path().join("trajectories"));
    }

    fn run_ev(run: &str, i: u64) -> Event {
        Event::new("graph", Level::Info, "run.journal")
            .field("i", i)
            .field("pad", "x".repeat(40))
            .in_run(RunId(run.into()))
    }

    fn count(d: &Path) -> usize {
        std::fs::read_dir(d.join("trajectories")).unwrap().count()
    }

    /// Spec 115 R2: with a tiny budget the directory is pruned oldest-first
    /// instead of growing, the prune is an event, and the run that is still
    /// open is never pruned.
    #[test]
    fn a_tiny_file_budget_prunes_oldest_first_and_keeps_the_open_run() {
        let d = tempfile::tempdir().unwrap();
        let rep = Arc::new(Recorder::new());
        let sink = TrajectorySink::new(d.path(), Arc::new(NoRedaction))
            .with_retention(Retention {
                max_bytes: None,
                max_files: Some(5),
            })
            .with_reporter(rep.clone());
        for n in 0..40 {
            let run = format!("r{n:03}");
            for i in 0..3 {
                sink.event(&run_ev(&run, i));
                assert!(count(d.path()) <= 5, "dir grew past the budget at {run}");
            }
            sink.event(&Event::new("graph", Level::Info, names::RUN_OUTCOME).in_run(RunId(run)));
        }
        assert!(!rep.events_named(names::TRAJECTORY_PRUNED).is_empty());
        // The newest run is intact: all 3 journal lines and its outcome.
        let newest = std::fs::read_to_string(d.path().join("trajectories/r039.jsonl")).unwrap();
        assert_eq!(newest.lines().count(), 4);
        // The oldest are gone, and what remains is the newest tail.
        assert!(!d.path().join("trajectories/r000.jsonl").exists());
    }

    /// Spec 115 R2: a byte budget smaller than one open run is reported once
    /// as over-budget and NOTHING of the open run is dropped.
    #[test]
    fn an_open_run_larger_than_the_budget_is_reported_not_dropped() {
        let d = tempfile::tempdir().unwrap();
        let rep = Arc::new(Recorder::new());
        let sink = TrajectorySink::new(d.path(), Arc::new(NoRedaction))
            .with_retention(Retention {
                max_bytes: Some(100),
                max_files: None,
            })
            .with_reporter(rep.clone());
        for i in 0..10 {
            sink.event(&run_ev("only", i));
        }
        let l = std::fs::read_to_string(d.path().join("trajectories/only.jsonl")).unwrap();
        assert_eq!(l.lines().count(), 10, "the open run must be intact");
        assert_eq!(rep.events_named(names::TRAJECTORY_OVER_BUDGET).len(), 1);
    }

    /// Spec 115 R2: the index is seeded from files already on disk (a daemon
    /// restart must not forget what the last one wrote).
    #[test]
    fn existing_files_count_against_the_budget_after_a_restart() {
        let d = tempfile::tempdir().unwrap();
        let tdir = d.path().join("trajectories");
        std::fs::create_dir_all(&tdir).unwrap();
        for n in 0..8 {
            std::fs::write(tdir.join(format!("old{n}.jsonl")), b"{}\n").unwrap();
        }
        let sink = TrajectorySink::new(d.path(), Arc::new(NoRedaction)).with_retention(Retention {
            max_bytes: None,
            max_files: Some(4),
        });
        sink.event(&run_ev("fresh", 0));
        assert!(count(d.path()) <= 4);
        assert!(tdir.join("fresh.jsonl").exists());
    }
}
