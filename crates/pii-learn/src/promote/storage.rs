//! Where promotions live and the one scope-checked write path (spec 125 R3,
//! R6): `<dir>/learn/promotions/`, `ledger.jsonl`, `promotion_status.json`.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};

use super::types::{
    LedgerEntry, Metric, Outcome, Promotion, PromotionKind, PromotionState, PromotionStatus,
};
use crate::scan::RunSummary;
use crate::store::{StoreError, area, read_small, write_atomic};

const MAX_PROMOTIONS: usize = 10_000;

pub(super) fn valid_promotion_id(id: &str) -> bool {
    id.strip_prefix("p-").is_some_and(|h| {
        h.len() == 16
            && h.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

pub(super) fn promotions_dir(dir: &Path) -> PathBuf {
    area(dir).join("promotions")
}

pub(super) fn ledger_path(dir: &Path) -> PathBuf {
    area(dir).join("ledger.jsonl")
}

pub(super) fn status_path(dir: &Path) -> PathBuf {
    area(dir).join("promotion_status.json")
}

/// The scope check (R6): a promotion writes only inside `<dir>/learn/`. A path
/// with a `..` component, outside that area, or that is a symlink is refused —
/// so no secret, gate, capability scope, observability file or source file can
/// be a promotion's write target. Every write in this module goes through it.
fn scoped(dir: &Path, path: &Path) -> Result<(), StoreError> {
    let root = area(dir);
    // Only what lies below the area root is checked for `..`: the root
    // itself may legitimately be spelled with one.
    let escapes = path.strip_prefix(&root).map_or(true, |rel| {
        !rel.components().all(|c| matches!(c, Component::Normal(_)))
    });
    if escapes {
        return Err(StoreError(format!(
            "learn: promotion write outside its own store area refused: {}",
            path.display()
        )));
    }
    if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(StoreError(format!(
            "learn: promotion write through a symlink refused: {}",
            path.display()
        )));
    }
    Ok(())
}

/// Scope-checked atomic write of `bytes` to `path`.
///
/// # Errors
/// [`StoreError`] when `path` is outside the promotion area or the write fails.
pub fn write_scoped(dir: &Path, path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    scoped(dir, path)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| StoreError(format!("create: {e}")))?;
    }
    write_atomic(path, bytes)
}

pub(super) fn append_ledger(dir: &Path, mut e: LedgerEntry) -> Result<u64, StoreError> {
    let path = ledger_path(dir);
    scoped(dir, &path)?;
    fs::create_dir_all(area(dir)).map_err(|x| StoreError(format!("create: {x}")))?;
    let seq = fs::read_to_string(&path).map_or(0, |t| t.lines().count() as u64) + 1;
    e.seq = seq;
    let mut line = serde_json::to_vec(&e).map_err(|x| StoreError(x.to_string()))?;
    line.push(b'\n');
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|x| StoreError(format!("ledger {}: {x}", path.display())))?;
    f.write_all(&line)
        .and_then(|()| f.sync_data())
        .map_err(|x| StoreError(format!("ledger {}: {x}", path.display())))?;
    Ok(seq)
}

/// The last `n` ledger entries, oldest first (unparsable lines are skipped).
#[must_use]
pub fn ledger_tail(dir: &Path, n: usize) -> Vec<LedgerEntry> {
    let Ok(text) = fs::read_to_string(ledger_path(dir)) else {
        return Vec::new();
    };
    let all: Vec<LedgerEntry> = text
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    let skip = all.len().saturating_sub(n);
    all.into_iter().skip(skip).collect()
}

pub(super) fn save(dir: &Path, p: &Promotion) -> Result<(), StoreError> {
    let bytes = serde_json::to_vec_pretty(p).map_err(|e| StoreError(e.to_string()))?;
    write_scoped(
        dir,
        &promotions_dir(dir).join(format!("{}.json", p.id)),
        &bytes,
    )
}

/// Every stored promotion, sorted by id.
#[must_use]
pub fn list_promotions(dir: &Path) -> Vec<Promotion> {
    let Ok(rd) = fs::read_dir(promotions_dir(dir)) else {
        return Vec::new();
    };
    let mut names: Vec<String> = rd
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.strip_suffix(".json").is_some_and(valid_promotion_id))
        .collect();
    names.sort();
    names
        .into_iter()
        .take(MAX_PROMOTIONS)
        .filter_map(|n| {
            let b = read_small(&promotions_dir(dir).join(n))?;
            serde_json::from_slice(&b).ok()
        })
        .collect()
}

/// One promotion by id.
#[must_use]
pub fn get_promotion(dir: &Path, id: &str) -> Option<Promotion> {
    if !valid_promotion_id(id) {
        return None;
    }
    let b = read_small(&promotions_dir(dir).join(format!("{id}.json")))?;
    serde_json::from_slice(&b).ok()
}

/// The named refusal for starting `graph`, if a promotion in effect at `now`
/// quarantines it. Read-only: this is what the engine asks before every run.
#[must_use]
pub fn quarantine_reason(dir: &Path, graph: &str, now: u64) -> Option<String> {
    list_promotions(dir)
        .into_iter()
        .find(|p| {
            p.in_effect(now)
                && match p.kind {
                    PromotionKind::QuarantineGraph => p.graph == graph,
                }
        })
        .map(|p| format!("learn: graph {graph} is quarantined by promotion {}", p.id))
}

/// Promotion counts by state plus `refused`; every key is always present.
#[must_use]
pub fn promotion_counts(dir: &Path) -> BTreeMap<String, u64> {
    let mut out: BTreeMap<String, u64> = PromotionState::ALL
        .iter()
        .map(|s| (s.as_str().to_owned(), 0))
        .collect();
    for p in list_promotions(dir) {
        *out.entry(p.state.as_str().to_owned()).or_default() += 1;
    }
    out.insert("refused".to_owned(), refused_total(dir));
    out
}

pub(super) fn read_status(dir: &Path) -> PromotionStatus {
    read_small(&status_path(dir))
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub(super) fn refused_total(dir: &Path) -> u64 {
    read_status(dir).refused_by_reason.values().sum()
}

/// Failed runs of `graph` whose trajectory was last written in
/// `[from, to)` (epoch seconds).
fn failed_in(runs: &[RunSummary], graph: &str, from: u64, to: u64) -> u64 {
    runs.iter()
        .filter(|r| {
            r.graph == graph && r.verdict == "failure" && r.mtime_secs >= from && r.mtime_secs < to
        })
        .count() as u64
}

/// Measures `p`'s metric over its window and the window before it (R5).
#[must_use]
pub fn measure(p: &Promotion, runs: &[RunSummary]) -> Outcome {
    let (before, after) = match p.metric {
        Metric::GraphFailedRuns => (
            failed_in(
                runs,
                &p.graph,
                p.applied_secs.saturating_sub(p.window_secs),
                p.applied_secs,
            ),
            failed_in(
                runs,
                &p.graph,
                p.applied_secs,
                p.applied_secs.saturating_add(p.window_secs),
            ),
        ),
    };
    Outcome {
        before,
        after,
        improved: after < before,
    }
}
