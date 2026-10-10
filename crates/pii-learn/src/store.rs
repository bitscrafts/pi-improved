//! Where candidates live: `<dir>/learn/candidates/<id>.json` plus
//! `<dir>/learn/status.json`. Written only by `pii learn scan`; the daemon and
//! the graph engine only ever read the status (spec 123 R8).

use std::collections::BTreeMap;
use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::candidate::{Candidate, is_valid_id};
use crate::scan::ScanStats;

/// Most candidate files `list` opens, and the largest one it reads.
const MAX_LIST_FILES: usize = 10_000;
const MAX_CANDIDATE_BYTES: u64 = 1024 * 1024;

/// What the last scan did, persisted so `pii health` and `pii metrics` can
/// report it without the daemon ever running an extraction.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LearnStatus {
    /// The scan root (candidates cite paths relative to it).
    pub root: String,
    /// What the last scan read and skipped.
    pub scan: ScanStats,
    /// Candidates emitted by the last scan, by kind.
    pub candidates_by_kind: BTreeMap<String, u64>,
    /// Candidates refused by the last scan, by reason.
    pub refused_by_reason: BTreeMap<String, u64>,
    /// Refusals across every scan so far.
    pub refused_total: u64,
}

/// A store failure, as text.
#[derive(Debug)]
pub struct StoreError(pub String);

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for StoreError {}

pub(crate) fn area(dir: &Path) -> PathBuf {
    dir.join("learn")
}

fn candidates_dir(dir: &Path) -> PathBuf {
    area(dir).join("candidates")
}

/// Writes `bytes` to `path` through a sibling temp file and a rename.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes).map_err(|e| StoreError(format!("write {}: {e}", tmp.display())))?;
    fs::rename(&tmp, path).map_err(|e| StoreError(format!("rename {}: {e}", path.display())))
}

/// Upserts `candidates` (same id = same bytes) and returns how many were written.
///
/// # Errors
/// [`StoreError`] when the area cannot be created or a file cannot be written.
pub fn put_candidates(dir: &Path, candidates: &[Candidate]) -> Result<usize, StoreError> {
    let cdir = candidates_dir(dir);
    fs::create_dir_all(&cdir).map_err(|e| StoreError(format!("create {}: {e}", cdir.display())))?;
    for c in candidates {
        let bytes = serde_json::to_vec_pretty(c).map_err(|e| StoreError(e.to_string()))?;
        write_atomic(&cdir.join(format!("{}.json", c.id)), &bytes)?;
    }
    Ok(candidates.len())
}

/// Reads one regular, bounded file; a symlink or an oversize file is `None`.
pub(crate) fn read_small(path: &Path) -> Option<Vec<u8>> {
    let meta = fs::symlink_metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_CANDIDATE_BYTES {
        return None;
    }
    let mut buf = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(MAX_CANDIDATE_BYTES)
        .read_to_end(&mut buf)
        .ok()?;
    Some(buf)
}

/// Every stored candidate, sorted by id, with a count of entries skipped
/// (unreadable, oversize, symlinked, unparsable, or past the file cap).
#[must_use]
pub fn list(dir: &Path) -> (Vec<Candidate>, u64) {
    let Ok(rd) = fs::read_dir(candidates_dir(dir)) else {
        return (Vec::new(), 0);
    };
    let mut files: Vec<String> = rd
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.strip_suffix(".json").is_some_and(is_valid_id))
        .collect();
    files.sort();
    let mut skipped = files.len().saturating_sub(MAX_LIST_FILES) as u64;
    let mut out = Vec::new();
    for f in files.into_iter().take(MAX_LIST_FILES) {
        let parsed = read_small(&candidates_dir(dir).join(&f))
            .and_then(|b| serde_json::from_slice::<Candidate>(&b).ok());
        match parsed {
            Some(c) => out.push(c),
            None => skipped += 1,
        }
    }
    (out, skipped)
}

/// One candidate by id.
///
/// # Errors
/// [`StoreError`] when the id is malformed (it names a file, so it is
/// validated before it touches a path), absent, or unreadable.
pub fn show(dir: &Path, id: &str) -> Result<Candidate, StoreError> {
    if !is_valid_id(id) {
        return Err(StoreError(format!("not a candidate id: {id}")));
    }
    let bytes = read_small(&candidates_dir(dir).join(format!("{id}.json")))
        .ok_or_else(|| StoreError(format!("no candidate {id}")))?;
    serde_json::from_slice(&bytes).map_err(|e| StoreError(format!("candidate {id}: {e}")))
}

/// The last scan's status; `Ok(None)` before any scan.
///
/// # Errors
/// [`StoreError`] when a status file exists but is unreadable or unparsable.
pub fn read_status(dir: &Path) -> Result<Option<LearnStatus>, StoreError> {
    let path = area(dir).join("status.json");
    if fs::symlink_metadata(&path).is_err() {
        return Ok(None);
    }
    let bytes =
        read_small(&path).ok_or_else(|| StoreError(String::from("status.json unreadable")))?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|e| StoreError(format!("status.json: {e}")))
}

/// Persists the status.
///
/// # Errors
/// [`StoreError`] when the file cannot be written.
pub fn write_status(dir: &Path, status: &LearnStatus) -> Result<(), StoreError> {
    fs::create_dir_all(area(dir)).map_err(|e| StoreError(e.to_string()))?;
    let bytes = serde_json::to_vec_pretty(status).map_err(|e| StoreError(e.to_string()))?;
    write_atomic(&area(dir).join("status.json"), &bytes)
}
