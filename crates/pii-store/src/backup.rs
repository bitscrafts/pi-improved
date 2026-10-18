//! Backup and restore of the daemon's redb files (spec 105 R3).
//!
//! **No-tear guarantee.** redb takes an exclusive file lock on open. A backup
//! opens each store with [`redb::Database::open`] and copies the file while
//! HOLDING that handle, so (a) a running daemon makes the open fail — the
//! backup refuses instead of copying a file with a writer mid-commit — and
//! (b) once the handle is held no writer exists anywhere. Each copy is then
//! re-opened as a redb database and hashed, so a torn or truncated copy is
//! an error, not a silent bad backup.
//!
//! Restore never deletes: an existing store is moved to `<name>.old-<ms>`.

use std::path::{Path, PathBuf};

use pii_abi::PiiError;
use serde::{Deserialize, Serialize};

/// The redb files a daemon keeps in its store directory.
pub const STORE_FILES: &[&str] = &[
    "store.redb",
    "memory.redb",
    "approval.redb",
    "heal.redb",
    "channel.redb",
    "timer.redb",
];

/// The manifest file name inside a backup directory.
pub const MANIFEST: &str = "MANIFEST.json";

/// One backed-up file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupEntry {
    /// File name inside the store directory.
    pub name: String,
    /// Size in bytes.
    pub bytes: u64,
    /// Format version recorded in the file (0 = unstamped).
    pub format_version: u64,
    /// blake3 of the file contents, hex.
    pub blake3: String,
}

/// What a backup contains.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// Backup time, ms since the epoch.
    pub created_ms: u64,
    /// The files, in [`STORE_FILES`] order.
    pub entries: Vec<BackupEntry>,
}

fn io(what: &str, path: &Path, e: impl std::fmt::Display) -> PiiError {
    PiiError::Internal(format!("backup: {what} {}: {e}", path.display()))
}

// reason: operator tooling stamps wall-clock time on a backup; this is not
// replayed run logic, so the deterministic `Ctx::now()` does not apply.
#[allow(clippy::disallowed_methods)]
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

/// Opens `path` as redb (taking its exclusive lock).
pub(crate) fn lock(path: &Path) -> Result<redb::Database, PiiError> {
    redb::Database::open(path).map_err(|e| {
        PiiError::Invalid(format!(
            "store '{}' cannot be opened exclusively ({e}); stop the daemon first",
            path.display()
        ))
    })
}

fn hash_file(path: &Path) -> Result<String, PiiError> {
    let bytes = std::fs::read(path).map_err(|e| io("read", path, e))?;
    Ok(crate::hash(&bytes))
}

/// Copies `from` to `to` through a temp name, so `to` is never half-written.
fn copy_atomic(from: &Path, to: &Path) -> Result<(), PiiError> {
    let tmp = to.with_extension("redb.part");
    std::fs::copy(from, &tmp).map_err(|e| io("copy", from, e))?;
    std::fs::File::open(&tmp)
        .and_then(|f| f.sync_all())
        .map_err(|e| io("sync", &tmp, e))?;
    std::fs::rename(&tmp, to).map_err(|e| io("rename", to, e))
}

/// Backs up every store file in `src_dir` into `dest`.
///
/// # Errors
/// [`PiiError::Invalid`] when a store is in use (daemon running) or `dest` is
/// a non-empty directory; [`PiiError::Internal`] on an I/O failure or a copy
/// that does not verify.
pub fn backup(src_dir: &Path, dest: &Path) -> Result<Manifest, PiiError> {
    if dest.is_dir()
        && std::fs::read_dir(dest)
            .map_err(|e| io("read", dest, e))?
            .next()
            .is_some()
    {
        return Err(PiiError::Invalid(format!(
            "backup destination '{}' is not empty",
            dest.display()
        )));
    }
    std::fs::create_dir_all(dest).map_err(|e| io("create", dest, e))?;
    let mut entries = Vec::new();
    for name in STORE_FILES {
        let src = src_dir.join(name);
        if !src.is_file() {
            continue;
        }
        let held = lock(&src)?;
        let out = dest.join(name);
        copy_atomic(&src, &out)?;
        let source_stamp = crate::identity::read_stamp(&held)?;
        drop(held);
        // Verify: the copy is a valid redb, carries the same stamp, hashes.
        let copy_stamp = crate::identity::read_stamp(&lock(&out)?)?;
        if copy_stamp != source_stamp {
            return Err(io("verify", &out, "format version differs from source"));
        }
        entries.push(BackupEntry {
            name: (*name).to_owned(),
            bytes: std::fs::metadata(&out)
                .map_err(|e| io("stat", &out, e))?
                .len(),
            format_version: source_stamp,
            blake3: hash_file(&out)?,
        });
    }
    if entries.is_empty() {
        return Err(PiiError::Invalid(format!(
            "no store files found in '{}'",
            src_dir.display()
        )));
    }
    let manifest = Manifest {
        created_ms: now_ms(),
        entries,
    };
    let json = serde_json::to_vec_pretty(&manifest)
        .map_err(|e| PiiError::Internal(format!("backup: manifest: {e}")))?;
    let mpath = dest.join(MANIFEST);
    std::fs::write(&mpath, json).map_err(|e| io("write", &mpath, e))?;
    Ok(manifest)
}

/// Restores a backup made by [`backup`] into `target_dir`.
///
/// Refuses when any file it would write already exists, unless `force`; with
/// `force` the existing file is MOVED to `<name>.old-<ms>` (never deleted).
/// Every backup file is hash-checked against the manifest before anything in
/// `target_dir` is touched.
///
/// # Errors
/// [`PiiError::Invalid`] on a guard refusal, a store in use, or a manifest
/// mismatch; [`PiiError::Internal`] on an I/O failure.
pub fn restore(src: &Path, target_dir: &Path, force: bool) -> Result<Manifest, PiiError> {
    let mpath = src.join(MANIFEST);
    let raw = std::fs::read(&mpath).map_err(|e| io("read", &mpath, e))?;
    let manifest: Manifest = serde_json::from_slice(&raw)
        .map_err(|e| PiiError::Invalid(format!("backup manifest '{}': {e}", mpath.display())))?;
    for e in &manifest.entries {
        if !STORE_FILES.contains(&e.name.as_str()) {
            return Err(PiiError::Invalid(format!("unexpected file '{}'", e.name)));
        }
        let got = hash_file(&src.join(&e.name))?;
        if got != e.blake3 {
            return Err(PiiError::Invalid(format!(
                "backup file '{}' does not match its manifest hash",
                e.name
            )));
        }
    }
    let existing: Vec<PathBuf> = manifest
        .entries
        .iter()
        .map(|e| target_dir.join(&e.name))
        .filter(|p| p.exists())
        .collect();
    if !existing.is_empty() && !force {
        let names: Vec<String> = existing.iter().map(|p| p.display().to_string()).collect();
        return Err(PiiError::Invalid(format!(
            "refusing to restore over existing stores ({}); pass --force to move them aside",
            names.join(", ")
        )));
    }
    for p in &existing {
        drop(lock(p)?);
    }
    std::fs::create_dir_all(target_dir).map_err(|e| io("create", target_dir, e))?;
    let stamp = now_ms();
    for p in &existing {
        let aside = PathBuf::from(format!("{}.old-{stamp}", p.display()));
        std::fs::rename(p, &aside).map_err(|e| io("move aside", p, e))?;
    }
    for e in &manifest.entries {
        copy_atomic(&src.join(&e.name), &target_dir.join(&e.name))?;
    }
    Ok(manifest)
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
    use crate::Store;
    use crate::approval::ApprovalStore;

    fn scratch() -> tempfile::TempDir {
        crate::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "backup-").unwrap()
    }

    fn seed(dir: &Path) {
        let store = Store::open(&dir.join("store.redb")).unwrap();
        let mut h = pii_abi::RunHeader {
            run: store.next_run_id().unwrap(),
            graph: "g".to_owned(),
            status: pii_abi::RunStatus::Running,
            seq: 0,
            dur_ms: 0,
            revision: String::new(),
            process_start_ms: 0,
            created_ms: 0,
        };
        let ev = pii_abi::RunEvent::Started {
            graph: "g".to_owned(),
            input_hash: "h".to_owned(),
        };
        store.append_event(&mut h, &ev).unwrap();
        drop(ApprovalStore::open(&dir.join("approval.redb")).unwrap());
    }

    #[test]
    fn round_trip_preserves_runs_and_guard_refuses_overwrite() {
        let d = scratch();
        let live = d.path().join("live");
        std::fs::create_dir_all(&live).unwrap();
        seed(&live);
        let bak = d.path().join("bak");
        let m = backup(&live, &bak).unwrap();
        assert_eq!(m.entries.len(), 2);
        assert!(m.entries.iter().all(|e| e.format_version == 1));
        // guard: restoring over the live (non-empty) dir refuses
        let err = restore(&bak, &live, false).unwrap_err().to_string();
        assert!(
            err.contains("refusing to restore") && err.contains("--force"),
            "{err}"
        );
        // move originals aside, restore into the empty dir
        let aside = d.path().join("aside");
        std::fs::rename(&live, &aside).unwrap();
        restore(&bak, &live, false).unwrap();
        let before = Store::open(&aside.join("store.redb"))
            .unwrap()
            .info()
            .unwrap();
        let after = Store::open(&live.join("store.redb"))
            .unwrap()
            .info()
            .unwrap();
        assert_eq!(before.tables, after.tables);
        assert_eq!(after.tables["run"], 1);
    }

    #[test]
    fn force_moves_aside_and_never_deletes() {
        let d = scratch();
        let live = d.path().join("live");
        std::fs::create_dir_all(&live).unwrap();
        seed(&live);
        let bak = d.path().join("bak");
        backup(&live, &bak).unwrap();
        restore(&bak, &live, true).unwrap();
        let olds = std::fs::read_dir(&live)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains(".redb.old-"))
            .count();
        assert_eq!(olds, 2, "both originals were moved aside, not deleted");
    }

    #[test]
    fn a_store_in_use_is_refused_not_copied() {
        let d = scratch();
        let live = d.path().join("live");
        std::fs::create_dir_all(&live).unwrap();
        let _open = Store::open(&live.join("store.redb")).unwrap();
        let bak = d.path().join("bak");
        let err = backup(&live, &bak).unwrap_err().to_string();
        assert!(err.contains("stop the daemon first"), "{err}");
        assert!(!bak.join("store.redb").exists());
    }

    #[test]
    fn a_corrupted_backup_file_fails_the_hash_check() {
        let d = scratch();
        let live = d.path().join("live");
        std::fs::create_dir_all(&live).unwrap();
        seed(&live);
        let bak = d.path().join("bak");
        backup(&live, &bak).unwrap();
        let f = bak.join("approval.redb");
        let mut b = std::fs::read(&f).unwrap();
        let last = b.len() - 1;
        b[last] ^= 0xff;
        std::fs::write(&f, b).unwrap();
        let err = restore(&bak, &d.path().join("t"), false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("manifest hash"), "{err}");
    }
}
