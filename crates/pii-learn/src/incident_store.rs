//! Spec 166: the incident store.
//!
//! Incidents opened from the runtime log are kept as `<dir>/incidents/<id>.json`.
//! The id is derived from fingerprint and source alone, so a re-ingest updates
//! the row instead of adding one.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::incident_runtime::RuntimeIncident;
use crate::store::{StoreError, read_small, write_atomic};

/// One stored incident.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredIncident {
    /// 16 hex digits of the digest of fingerprint and source.
    pub id: String,
    /// What happened.
    pub fingerprint: String,
    /// The log it was read from.
    pub source: String,
    /// Occurrences folded into this incident.
    pub count: u64,
    /// Unix seconds of the first event of the evidence.
    pub first_seen: i64,
    /// Unix seconds of the last event of the evidence.
    pub last_seen: i64,
    /// Unix seconds at which the condition became detectable.
    pub opened_at: i64,
}

/// The id of the incident with this fingerprint and source.
#[must_use]
pub fn incident_id(fingerprint: &str, source: &str) -> String {
    let mut h = blake3::Hasher::new();
    h.update(fingerprint.as_bytes());
    h.update(&[0]);
    h.update(source.as_bytes());
    let hex = h.finalize().to_hex();
    hex.get(..16).unwrap_or_default().to_owned()
}

fn is_valid_id(id: &str) -> bool {
    id.len() == 16
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn incidents_dir(dir: &Path) -> PathBuf {
    dir.join("incidents")
}

/// Writes every incident (same id = same row) and returns what was stored.
///
/// # Errors
/// [`StoreError`] when the area cannot be created or a file cannot be written.
pub fn ingest(dir: &Path, found: &[RuntimeIncident]) -> Result<Vec<StoredIncident>, StoreError> {
    let idir = incidents_dir(dir);
    let mut out = Vec::new();
    for i in found {
        if out.is_empty() {
            fs::create_dir_all(&idir)
                .map_err(|e| StoreError(format!("create {}: {e}", idir.display())))?;
        }
        let s = StoredIncident {
            id: incident_id(&i.fingerprint, &i.source),
            fingerprint: i.fingerprint.clone(),
            source: i.source.clone(),
            count: i.count,
            first_seen: i.first_seen,
            last_seen: i.last_seen,
            opened_at: i.opened_at,
        };
        let bytes = serde_json::to_vec_pretty(&s).map_err(|e| StoreError(e.to_string()))?;
        write_atomic(&idir.join(format!("{}.json", s.id)), &bytes)?;
        out.push(s);
    }
    Ok(out)
}

/// One stored incident by id.
///
/// # Errors
/// [`StoreError`] when the id is malformed (it names a file), absent or unreadable.
pub fn show(dir: &Path, id: &str) -> Result<StoredIncident, StoreError> {
    if !is_valid_id(id) {
        return Err(StoreError(format!("not an incident id: {id}")));
    }
    let bytes = read_small(&incidents_dir(dir).join(format!("{id}.json")))
        .ok_or_else(|| StoreError(format!("no incident {id}")))?;
    serde_json::from_slice(&bytes).map_err(|e| StoreError(format!("incident {id}: {e}")))
}
