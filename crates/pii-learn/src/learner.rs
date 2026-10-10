//! One extraction scan, its events, and the `learn` health seam (spec 123 R7).
//!
//! A scan is a short-lived process (`pii learn scan`), never a task inside the
//! daemon: it cannot block the daemon, and its cost is bounded by the
//! `LearnConfig` read caps. The daemon only reads the status it leaves behind.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use pii_observe::{Event, Health, Level, Observer, State, SubsystemHealth, names};

use crate::candidate::Kind;
use crate::config::LearnConfig;
use crate::extract::extract_all;
use crate::scan::scan;
use crate::store::{self, LearnStatus};

const SUBSYSTEM: &str = "learn";

/// Runs scans and reports them.
pub struct Learner {
    cfg: LearnConfig,
    obs: Arc<dyn Observer>,
}

impl Learner {
    /// A learner with thresholds `cfg`, reporting to `obs`.
    #[must_use]
    pub fn new(cfg: LearnConfig, obs: Arc<dyn Observer>) -> Self {
        Self { cfg, obs }
    }

    /// Scans `<root>/trajectories`, writes the candidates and the status under
    /// `<dir>/learn`, and emits the three `learn.*` events. Proposes only.
    ///
    /// # Errors
    /// A message when the root cannot be scanned or the store cannot be written.
    pub fn run_scan(&self, root: &Path, dir: &Path) -> Result<LearnStatus, String> {
        let (runs, stats) = scan(root, &self.cfg).map_err(|e| e.to_string())?;
        let ex = extract_all(&runs, &self.cfg);
        let mut by_kind: BTreeMap<String, u64> = Kind::ALL
            .iter()
            .map(|k| (k.as_str().to_owned(), 0))
            .collect();
        for c in &ex.candidates {
            *by_kind.entry(c.kind.as_str().to_owned()).or_default() += 1;
        }
        let refused: BTreeMap<String, u64> = ex
            .refused
            .iter()
            .map(|(r, n)| (r.as_str().to_owned(), *n))
            .collect();
        store::put_candidates(dir, &ex.candidates).map_err(|e| e.to_string())?;
        let prior = store::read_status(dir)
            .ok()
            .flatten()
            .map_or(0, |s| s.refused_total);
        let out = LearnStatus {
            root: root.display().to_string(),
            scan: stats,
            candidates_by_kind: by_kind,
            refused_by_reason: refused,
            refused_total: prior + ex.refused.values().sum::<u64>(),
        };
        store::write_status(dir, &out).map_err(|e| e.to_string())?;
        self.report(&out);
        Ok(out)
    }

    /// Emits the three `learn.*` events for a finished scan's `status`.
    pub fn report(&self, s: &LearnStatus) {
        let mut e = Event::new(SUBSYSTEM, Level::Info, names::LEARN_CANDIDATES_EXTRACTED);
        for k in Kind::ALL {
            e = e.field(
                k.as_str(),
                s.candidates_by_kind.get(k.as_str()).copied().unwrap_or(0),
            );
        }
        self.obs.event(&e);
        for (reason, n) in &s.refused_by_reason {
            self.obs.event(
                &Event::new(SUBSYSTEM, Level::Warn, names::LEARN_CANDIDATE_REFUSED)
                    .field("reason", reason.as_str())
                    .field("count", *n),
            );
        }
        self.obs.event(
            &Event::new(SUBSYSTEM, Level::Info, names::LEARN_SCAN_COMPLETED)
                .field("files_scanned", s.scan.files_scanned)
                .field("files_seen", s.scan.files_seen)
                .field("bytes_read", s.scan.bytes_read)
                .field("corrupt_lines", s.scan.corrupt_lines)
                .field("duration_ms", s.scan.duration_ms),
        );
    }
}

/// The `learn` health seam: reads the status the last scan left under `dir`.
pub struct LearnHealth {
    dir: PathBuf,
}

impl LearnHealth {
    /// Health over the store directory `dir` (the one holding `learn/`).
    #[must_use]
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The last scan's status, if any (what `pii metrics` reads).
    #[must_use]
    pub fn status(&self) -> Option<LearnStatus> {
        store::read_status(&self.dir).ok().flatten()
    }
}

impl Health for LearnHealth {
    fn health(&self) -> SubsystemHealth {
        let mut h = SubsystemHealth::ok(SUBSYSTEM);
        match store::read_status(&self.dir) {
            Ok(None) => h.detail = Some(String::from("no scan yet (pii learn scan)")),
            Err(e) => {
                h.state = State::Degraded;
                h.detail = Some(e.to_string());
            }
            Ok(Some(s)) => {
                let total: u64 = s.candidates_by_kind.values().sum();
                h.identity.insert("candidates".into(), total.to_string());
                h.identity
                    .insert("files_scanned".into(), s.scan.files_scanned.to_string());
                h.identity
                    .insert("refused_total".into(), s.refused_total.to_string());
                h.identity.insert("root".into(), s.root);
            }
        }
        h
    }
}
