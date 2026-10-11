//! The unattended half of the loop (spec 139 R1, R4): one pass scans the
//! trajectories, extracts candidates and shadow-verifies the new ones, with no
//! operator. It proposes and verifies; it never promotes — promotion stays a
//! human act through the daemon (`learn.promote`), and this module does not
//! even link to [`crate::Promoter`].
//!
//! Bounded by construction: candidates and verdicts are keyed by candidate id
//! (a lesson seen again rewrites the same file), a candidate that already has
//! a verdict is not re-verified, and the status here is one fixed-size file.
//! The ledger only grows on a human's promotion.

use std::path::Path;
use std::sync::Arc;

use pii_observe::{Event, Level, Observer, names};
use serde::{Deserialize, Serialize};

use crate::config::LearnConfig;
use crate::extract::extract_all;
use crate::scan::scan;
use crate::store::{self, StoreError, area, read_small, write_atomic};
use crate::verdict::get_verdict;
use crate::verify::{Verifier, VerifyConfig};

const SUBSYSTEM: &str = "learn";

/// What one unattended pass needs.
#[derive(Debug, Clone)]
pub struct UnattendedConfig {
    /// Extraction thresholds and read bounds.
    pub learn: LearnConfig,
    /// Shadow-run budgets and locations.
    pub verify: VerifyConfig,
}

/// The unattended loop's record, `<dir>/learn/unattended.json`: what `pii
/// metrics` reads so a silent fortnight shows as zeros, not as nothing.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct UnattendedStatus {
    /// Passes completed since the loop was enabled.
    pub runs: u64,
    /// Distinct candidates first seen by an unattended pass, cumulative.
    pub candidates_proposed_total: u64,
    /// Epoch seconds of the last pass.
    pub last_at_secs: u64,
    /// Candidates the last pass extracted (new and already known).
    pub last_candidates: u64,
    /// Candidates the last pass verified (those without a verdict).
    pub last_verified: u64,
}

/// The last unattended pass's status; `None` before any pass or when unreadable.
#[must_use]
pub fn read_unattended_status(dir: &Path) -> Option<UnattendedStatus> {
    serde_json::from_slice(&read_small(&area(dir).join("unattended.json"))?).ok()
}

fn write_status(dir: &Path, s: &UnattendedStatus) -> Result<(), StoreError> {
    std::fs::create_dir_all(area(dir)).map_err(|e| StoreError(e.to_string()))?;
    let bytes = serde_json::to_vec_pretty(s).map_err(|e| StoreError(e.to_string()))?;
    write_atomic(&area(dir).join("unattended.json"), &bytes)
}

/// One extraction + verification pass over `root`, recorded under `dir`.
/// Emits [`names::LEARN_UNATTENDED_RUN`] even when nothing was found.
///
/// # Errors
/// A message when the trajectories cannot be scanned or the store cannot be written.
pub fn run_unattended(
    root: &Path,
    dir: &Path,
    cfg: &UnattendedConfig,
    obs: &Arc<dyn Observer>,
    now_secs: u64,
) -> Result<UnattendedStatus, String> {
    let (traj, scanned) = scan(root, &cfg.learn).map_err(|e| e.to_string())?;
    let ex = extract_all(&traj, &cfg.learn);
    let known: std::collections::BTreeSet<String> =
        store::list(dir).0.into_iter().map(|c| c.id).collect();
    let new = ex
        .candidates
        .iter()
        .filter(|c| !known.contains(&c.id))
        .count() as u64;
    store::put_candidates(dir, &ex.candidates).map_err(|e| e.to_string())?;
    // The scan's own status and events, exactly as `pii learn scan` leaves them.
    let by_kind = crate::Kind::ALL
        .iter()
        .map(|k| {
            let n = ex.candidates.iter().filter(|c| c.kind == *k).count() as u64;
            (k.as_str().to_owned(), n)
        })
        .collect();
    let prior = store::read_status(dir)
        .ok()
        .flatten()
        .map_or(0, |s| s.refused_total);
    let status = store::LearnStatus {
        root: root.display().to_string(),
        scan: scanned,
        candidates_by_kind: by_kind,
        refused_by_reason: ex
            .refused
            .iter()
            .map(|(r, n)| (r.as_str().to_owned(), *n))
            .collect(),
        refused_total: prior + ex.refused.values().sum::<u64>(),
    };
    store::write_status(dir, &status).map_err(|e| e.to_string())?;
    crate::Learner::new(cfg.learn, Arc::clone(obs)).report(&status);

    let todo: Vec<_> = ex
        .candidates
        .iter()
        .filter(|c| get_verdict(dir, &c.id).is_none())
        .cloned()
        .collect();
    if !todo.is_empty() {
        Verifier::new(cfg.verify.clone(), Arc::clone(obs)).verify_all(dir, &todo)?;
    }

    let prev = read_unattended_status(dir).unwrap_or_default();
    let out = UnattendedStatus {
        runs: prev.runs + 1,
        candidates_proposed_total: prev.candidates_proposed_total + new,
        last_at_secs: now_secs,
        last_candidates: ex.candidates.len() as u64,
        last_verified: todo.len() as u64,
    };
    write_status(dir, &out).map_err(|e| e.to_string())?;
    obs.event(
        &Event::new(SUBSYSTEM, Level::Info, names::LEARN_UNATTENDED_RUN)
            .field("candidates", out.last_candidates)
            .field("new_candidates", new)
            .field("verified", out.last_verified)
            .field("total_proposed", out.candidates_proposed_total),
    );
    Ok(out)
}
