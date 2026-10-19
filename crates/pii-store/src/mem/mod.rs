//! `MemStore` (F4.2): the memory index on a second redb file.
//!
//! The vector index is isolated from the run journal because redb's lock is
//! per file. `redb` stays confined to this crate (xtask deps-check invariant
//! 5). This module holds the type, its wire types, and the lifecycle/meta
//! methods; [`write`] holds the write APIs and [`read`] the read APIs.

mod read;
mod write;

use std::path::Path;

use pii_abi::PiiError;
use redb::{Database, ReadableDatabase, ReadableTable};

use crate::tables::{
    MEM_ARCHIVE, MEM_DF, MEM_DOC, MEM_EDGE, MEM_EVIDENCE, MEM_META, MEM_TERM, MEM_VEC,
};

/// The memory index store (F4.2). All keys are strings; values are codec
/// bytes or counts. Open/commit happens inside each method — no transaction
/// is ever held across a call boundary.
pub struct MemStore {
    db: Database,
    path: std::path::PathBuf,
}

fn err(name: &str, e: impl std::fmt::Display) -> PiiError {
    PiiError::Internal(format!("memstore: open {name}: {e}"))
}

/// A put request: the encoded doc plus its index rows (precomputed by
/// `pii-memory` so `pii-store` stays a dumb KV layer).
pub struct PutRows {
    /// Doc id (blake3(text)[..16] hex).
    pub id: String,
    /// Encoded `Doc`.
    pub doc: Vec<u8>,
    /// `(term, tf)` postings for this doc.
    pub terms: Vec<(String, u32)>,
    /// i8-quantised vector + f32 scale.
    pub vector: Option<(Vec<i8>, f32)>,
    /// Concept co-occurrence edges for this doc.
    pub edges: Vec<(String, u64)>,
    /// Token count of the doc text (dl for BM25).
    pub dl: u32,
    /// Layer tag stored in the additive `mem_layer` table (u8; 0 = L1).
    pub layer: u8,
}

/// One posting row read back from `mem_term`.
pub struct Posting {
    /// The doc id.
    pub doc: String,
    /// Term frequency.
    pub tf: u32,
}

impl MemStore {
    /// The file this memory store was opened from.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Opens (creating if needed) the memory database at `path`.
    ///
    /// # Errors
    /// [`PiiError::Internal`] when the file cannot be opened or a table
    /// cannot be defined.
    pub fn open(path: &Path) -> Result<Self, PiiError> {
        // RSS-gate fix (2026-09-05): redb defaults to a 1 GiB cache; this
        // store is small (approvals/channels/heal/memory/timers), so 4 MiB
        // is ample and avoids each auxiliary store compounding the
        // daemon's total cache footprint under soak (see pii-store/src/
        // lib.rs's Store::open for the full rationale).
        let db = Database::builder()
            .set_cache_size(4 * 1024 * 1024)
            .create(path)
            .map_err(|e| PiiError::Internal(format!("memstore: open {}: {e}", path.display())))?;
        {
            let write = db
                .begin_write()
                .map_err(|e| PiiError::Internal(format!("memstore: begin_write: {e}")))?;
            let names = [
                "mem_doc",
                "mem_term",
                "mem_df",
                "mem_vec",
                "mem_edge",
                "mem_meta",
                "mem_archive",
                "mem_evidence",
            ];
            write.open_table(MEM_DOC).map_err(|e| err("mem_doc", e))?;
            write.open_table(MEM_TERM).map_err(|e| err("mem_term", e))?;
            write.open_table(MEM_DF).map_err(|e| err("mem_df", e))?;
            write.open_table(MEM_VEC).map_err(|e| err("mem_vec", e))?;
            write.open_table(MEM_EDGE).map_err(|e| err("mem_edge", e))?;
            write.open_table(MEM_META).map_err(|e| err("mem_meta", e))?;
            write
                .open_table(MEM_ARCHIVE)
                .map_err(|e| err("mem_archive", e))?;
            write
                .open_table(MEM_EVIDENCE)
                .map_err(|e| err("mem_evidence", e))?;
            let _ = names;
            write
                .commit()
                .map_err(|e| PiiError::Internal(format!("memstore: commit tables: {e}")))?;
        }
        crate::identity::stamp(&db, path)?;
        Ok(Self {
            db,
            path: path.to_path_buf(),
        })
    }

    /// The persisted format version (spec 105).
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb failure.
    pub fn format_version(&self) -> Result<u64, PiiError> {
        crate::identity::read_stamp(&self.db)
    }

    /// `n_docs` from `mem_meta` (0 when unset).
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb failure.
    pub fn n_docs(&self) -> Result<u64, PiiError> {
        self.meta("n_docs")
    }

    /// `sum_dl` from `mem_meta` (0 when unset).
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb failure.
    pub fn sum_dl(&self) -> Result<u64, PiiError> {
        self.meta("sum_dl")
    }

    /// Number of distinct dims recorded in `mem_meta` (0 = none).
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb failure.
    pub fn dims(&self) -> Result<u64, PiiError> {
        self.meta("dims")
    }

    fn meta(&self, key: &str) -> Result<u64, PiiError> {
        let read = self
            .db
            .begin_read()
            .map_err(|e| PiiError::Internal(format!("memstore: begin_read: {e}")))?;
        let table = read
            .open_table(MEM_META)
            .map_err(|e| PiiError::Internal(format!("memstore: open mem_meta: {e}")))?;
        Ok(table.get(key).ok().flatten().map_or(0, |v| v.value()))
    }

    fn bump_meta(&self, key: &str, by: i64) -> Result<(), PiiError> {
        let write = self
            .db
            .begin_write()
            .map_err(|e| PiiError::Internal(format!("memstore: begin_write: {e}")))?;
        {
            let mut table = write
                .open_table(MEM_META)
                .map_err(|e| PiiError::Internal(format!("memstore: open mem_meta: {e}")))?;
            let cur = table.get(key).ok().flatten().map_or(0u64, |v| v.value());
            let next = if by >= 0 {
                cur.saturating_add(u64::try_from(by).unwrap_or(u64::MAX))
            } else {
                cur.saturating_sub(u64::try_from(-by).unwrap_or(u64::MAX))
            };
            table
                .insert(key, next)
                .map_err(|e| PiiError::Internal(format!("memstore: insert {key}: {e}")))?;
        }
        write
            .commit()
            .map_err(|e| PiiError::Internal(format!("memstore: commit meta: {e}")))
    }
}
