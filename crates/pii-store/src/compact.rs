//! Offline store compaction (spec 115 R3b).
//!
//! redb reuses freed pages and trims unused space at the END of the file, but
//! free pages in the middle of the file stay allocated, so a store that had
//! data deleted (a closed session's transcript, say) keeps its size.
//! [`compact`] asks redb to move live pages down and truncate the file. This
//! is redb's own crash-safe in-place compaction, not a copy-and-swap.
//! It is honest about what it did: [`Report`] carries the file sizes before
//! and after, measured with `stat`, not an estimate.
//!
//! What it does NOT do: it cannot shrink data that is still live. The main
//! store never deletes runs or journal events, so on a store that has only
//! grown (a soak, say) there are almost no free pages and the report will
//! show a small reduction or none. That is the answer, not a failure.
//!
//! Like backup, it takes redb's exclusive file lock first: a running daemon
//! makes the open fail and compaction refuses.

use std::path::Path;

use pii_abi::PiiError;

/// What a compaction did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Report {
    /// File size before, bytes.
    pub bytes_before: u64,
    /// File size after, bytes.
    pub bytes_after: u64,
}

/// Compacts the redb file at `path`.
///
/// # Errors
/// [`PiiError::Invalid`] when the store is in use (daemon running);
/// [`PiiError::Internal`] on an I/O or compaction failure.
pub fn compact(path: &Path) -> Result<Report, PiiError> {
    let size = |p: &Path| {
        std::fs::metadata(p)
            .map(|m| m.len())
            .map_err(|e| PiiError::Internal(format!("compact: stat {}: {e}", p.display())))
    };
    let bytes_before = size(path)?;
    let mut db = redb::Database::open(path).map_err(|e| {
        PiiError::Invalid(format!(
            "store '{}' cannot be opened exclusively ({e}); stop the daemon first",
            path.display()
        ))
    })?;
    // redb compacts in bounded steps; `true` means it moved something and a
    // further pass may free more. The bound only guards against a runaway.
    for _ in 0..64 {
        let moved = db
            .compact()
            .map_err(|e| PiiError::Internal(format!("compact {}: {e}", path.display())))?;
        if !moved {
            break;
        }
    }
    drop(db);
    Ok(Report {
        bytes_before,
        bytes_after: size(path)?,
    })
}

/// Compacts every store file in `dir` (the names of
/// [`crate::backup::STORE_FILES`] that exist). Refuses up front, touching
/// nothing, if ANY of them is held open.
///
/// # Errors
/// As [`compact`]; [`PiiError::Invalid`] when `dir` holds no store file.
pub fn compact_dir(dir: &Path) -> Result<Vec<(String, Report)>, PiiError> {
    let present: Vec<_> = crate::backup::STORE_FILES
        .iter()
        .filter(|n| dir.join(n).is_file())
        .collect();
    if present.is_empty() {
        return Err(PiiError::Invalid(format!(
            "no store files found in '{}'",
            dir.display()
        )));
    }
    // Sizes are taken BEFORE anything opens a file: redb itself trims unused
    // tail space when it opens or closes a database, so a `stat` after the
    // lock probe would already be smaller than what the operator had.
    let mut before = Vec::new();
    for n in &present {
        before.push(
            std::fs::metadata(dir.join(n))
                .map_err(|e| PiiError::Internal(format!("compact: stat {n}: {e}")))?
                .len(),
        );
    }
    for n in &present {
        // Probe the lock on all of them before changing any.
        drop(crate::backup::lock(&dir.join(n))?);
    }
    present
        .into_iter()
        .zip(before)
        .map(|(n, bytes_before)| {
            compact(&dir.join(n)).map(|r| ((*n).to_owned(), Report { bytes_before, ..r }))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::compact;
    use crate::Store;

    fn scratch() -> tempfile::TempDir {
        crate::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "compact-").unwrap()
    }

    /// Fills a store with session transcripts, then deletes half of them: the
    /// file keeps its size until it is compacted.
    fn store_with_freed_pages(path: &std::path::Path) {
        let store = Store::open(path).unwrap();
        for s in 0..20 {
            let id = format!("s{s}");
            store.put_session(&id, b"x").unwrap();
            for _ in 0..10 {
                store
                    .append_session_message(&id, &vec![7u8; 40_000])
                    .unwrap();
            }
        }
        // Delete every other session: the freed pages sit BETWEEN live ones,
        // which is the case redb cannot hand back by itself.
        for s in (0..20).step_by(2) {
            store.del_session(&format!("s{s}")).unwrap();
        }
    }

    #[test]
    fn compaction_really_shrinks_a_store_that_has_freed_pages() {
        let d = scratch();
        let path = d.path().join("store.redb");
        store_with_freed_pages(&path);
        let before = std::fs::metadata(&path).unwrap().len();
        let report = compact(&path).unwrap();
        let after = std::fs::metadata(&path).unwrap().len();
        assert_eq!(report.bytes_before, before);
        assert_eq!(report.bytes_after, after, "the report is a measurement");
        assert!(
            after * 10 < before * 7,
            "freed pages must be returned: {before} -> {after}"
        );
        // Still a valid store afterwards.
        let store = Store::open(&path).unwrap();
        store.put_session("again", b"ok").unwrap();
        assert_eq!(store.get_session("again").unwrap().unwrap(), b"ok");
    }

    #[test]
    fn compaction_refuses_while_the_store_is_open() {
        let d = scratch();
        let path = d.path().join("store.redb");
        let _held = Store::open(&path).unwrap();
        let err = compact(&path).unwrap_err();
        assert!(err.to_string().contains("stop the daemon"), "{err}");
    }
}
