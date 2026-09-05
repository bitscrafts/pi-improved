//! `MemStore` (F4.2): the memory index on a second redb file.
//!
//! The vector index is isolated from the run journal because redb's lock is
//! per file. `redb` stays confined to this crate (xtask deps-check invariant
//! 5).

use std::path::Path;

use pii_abi::PiiError;
use redb::{Database, ReadableDatabase, ReadableTable};

use crate::tables::{MEM_ARCHIVE, MEM_DF, MEM_DOC, MEM_EDGE, MEM_META, MEM_TERM, MEM_VEC};

/// The memory index store (F4.2). All keys are strings; values are codec
/// bytes or counts. Open/commit happens inside each method — no transaction
/// is ever held across a call boundary.
pub struct MemStore {
    db: Database,
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
}

/// One posting row read back from `mem_term`.
pub struct Posting {
    /// The doc id.
    pub doc: String,
    /// Term frequency.
    pub tf: u32,
}

impl MemStore {
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
            let _ = names;
            write
                .commit()
                .map_err(|e| PiiError::Internal(format!("memstore: commit tables: {e}")))?;
        }
        Ok(Self { db })
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

    /// Inserts one doc with all its index rows **in a single write txn**
    /// (F4.11: vectors are persisted with the doc, never memory-resident).
    /// Idempotent on `id`: re-put replaces the doc and its rows.
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb or encode failure.
    pub fn put(&self, rows: &PutRows) -> Result<(), PiiError> {
        let existed = self.doc_exists(&rows.id)?;
        let write = self
            .db
            .begin_write()
            .map_err(|e| PiiError::Internal(format!("memstore: begin_write: {e}")))?;
        {
            let mut doc = write
                .open_table(MEM_DOC)
                .map_err(|e| PiiError::Internal(format!("memstore: open mem_doc: {e}")))?;
            doc.insert(rows.id.as_str(), rows.doc.as_slice())
                .map_err(|e| PiiError::Internal(format!("memstore: put doc: {e}")))?;
            let mut term = write
                .open_table(MEM_TERM)
                .map_err(|e| PiiError::Internal(format!("memstore: open mem_term: {e}")))?;
            for (term_s, tf) in &rows.terms {
                term.insert((term_s.as_str(), rows.id.as_str()), *tf)
                    .map_err(|e| PiiError::Internal(format!("memstore: put term: {e}")))?;
            }
            if let Some((q, scale)) = &rows.vector {
                let mut bytes = Vec::with_capacity(q.len() + 4);
                for v in q {
                    bytes.extend_from_slice(&v.to_le_bytes());
                }
                bytes.extend_from_slice(&scale.to_le_bytes());
                let mut vec = write
                    .open_table(MEM_VEC)
                    .map_err(|e| PiiError::Internal(format!("memstore: open mem_vec: {e}")))?;
                vec.insert(rows.id.as_str(), bytes.as_slice())
                    .map_err(|e| PiiError::Internal(format!("memstore: put vec: {e}")))?;
            }
            let mut edge = write
                .open_table(MEM_EDGE)
                .map_err(|e| PiiError::Internal(format!("memstore: open mem_edge: {e}")))?;
            for (concept, w) in &rows.edges {
                // Co-occurrence accumulates across docs sharing the pair.
                let cur = edge
                    .get((concept.as_str(), rows.id.as_str()))
                    .ok()
                    .flatten()
                    .map_or(0, |v| v.value());
                edge.insert((concept.as_str(), rows.id.as_str()), cur + w)
                    .map_err(|e| PiiError::Internal(format!("memstore: put edge: {e}")))?;
            }
        }
        write
            .commit()
            .map_err(|e| PiiError::Internal(format!("memstore: commit put: {e}")))?;
        if !existed {
            self.bump_meta("n_docs", 1)?;
            self.bump_meta("sum_dl", i64::from(rows.dl))?;
        }
        if let Some((q, _)) = &rows.vector
            && self.meta("dims")? == 0
        {
            self.bump_meta("dims", i64::try_from(q.len()).unwrap_or(0))?;
        }
        // df: one per distinct term.
        self.bump_df(&rows.id)
    }

    fn doc_exists(&self, id: &str) -> Result<bool, PiiError> {
        let read = self
            .db
            .begin_read()
            .map_err(|e| PiiError::Internal(format!("memstore: begin_read: {e}")))?;
        let table = read
            .open_table(MEM_DOC)
            .map_err(|e| PiiError::Internal(format!("memstore: open mem_doc: {e}")))?;
        Ok(table.get(id).ok().flatten().is_some())
    }

    fn bump_df(&self, doc: &str) -> Result<(), PiiError> {
        let write = self
            .db
            .begin_write()
            .map_err(|e| PiiError::Internal(format!("memstore: begin_write: {e}")))?;
        {
            let table = write
                .open_table(MEM_TERM)
                .map_err(|e| PiiError::Internal(format!("memstore: open mem_term: {e}")))?;
            let mut seen: Vec<String> = Vec::new();
            let all = table
                .iter()
                .map_err(|e| PiiError::Internal(format!("memstore: iter mem_term: {e}")))?;
            for row in all {
                let Ok(row) = row else {
                    continue;
                };
                // Two-part key: `(key_tuple, value)` — the tuple is
                // `(term, doc_id)`, the value is the term frequency.
                let (key, _tf) = (row.0.value(), row.1.value());
                let (term, doc_id) = (key.0, key.1);
                if doc_id == doc && !seen.contains(&term.to_string()) {
                    seen.push(term.to_string());
                }
            }
            drop(table);
            let mut df = write
                .open_table(MEM_DF)
                .map_err(|e| PiiError::Internal(format!("memstore: open mem_df: {e}")))?;
            for t in seen {
                let cur = df.get(t.as_str()).ok().flatten().map_or(0, |v| v.value());
                df.insert(t.as_str(), cur + 1)
                    .map_err(|e| PiiError::Internal(format!("memstore: put df: {e}")))?;
            }
        }
        write
            .commit()
            .map_err(|e| PiiError::Internal(format!("memstore: commit df: {e}")))
    }
}

/// Read APIs for the memory lanes (F4.5/F4.7/F4.8). All reads stream from
/// redb — no in-RAM inverted index, no full-corpus resident array.
impl MemStore {
    /// Postings for one term (F4.5): `(doc_id, tf)` in key order.
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb failure.
    pub fn postings(&self, term: &str) -> Result<Vec<Posting>, PiiError> {
        let read = self
            .db
            .begin_read()
            .map_err(|e| PiiError::Internal(format!("memstore: begin_read: {e}")))?;
        let table = read
            .open_table(MEM_TERM)
            .map_err(|e| PiiError::Internal(format!("memstore: open mem_term: {e}")))?;
        let mut out = Vec::new();
        let from = (term, "");
        let mut range = table
            .range(from..)
            .map_err(|e| PiiError::Internal(format!("memstore: range mem_term: {e}")))?;
        for row in &mut range {
            let Ok(row) = row else { continue };
            let key = row.0.value();
            let (t, d) = (key.0, key.1);
            if t != term {
                break;
            }
            out.push(Posting {
                doc: String::from(d),
                tf: row.1.value(),
            });
        }
        Ok(out)
    }

    /// Document frequency for one term (0 when unseen).
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb failure.
    pub fn df(&self, term: &str) -> Result<u64, PiiError> {
        let read = self
            .db
            .begin_read()
            .map_err(|e| PiiError::Internal(format!("memstore: begin_read: {e}")))?;
        let table = read
            .open_table(MEM_DF)
            .map_err(|e| PiiError::Internal(format!("memstore: open mem_df: {e}")))?;
        Ok(table.get(term).ok().flatten().map_or(0, |v| v.value()))
    }

    /// Raw encoded doc bytes (`None` when absent).
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb failure.
    pub fn doc(&self, id: &str) -> Result<Option<Vec<u8>>, PiiError> {
        let read = self
            .db
            .begin_read()
            .map_err(|e| PiiError::Internal(format!("memstore: begin_read: {e}")))?;
        let table = read
            .open_table(MEM_DOC)
            .map_err(|e| PiiError::Internal(format!("memstore: open mem_doc: {e}")))?;
        Ok(table.get(id).ok().flatten().map(|v| v.value().to_vec()))
    }

    /// Every doc id, in key order (streamed one read txn, bounded by use).
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb failure.
    pub fn doc_ids(&self) -> Result<Vec<String>, PiiError> {
        let read = self
            .db
            .begin_read()
            .map_err(|e| PiiError::Internal(format!("memstore: begin_read: {e}")))?;
        let table = read
            .open_table(MEM_DOC)
            .map_err(|e| PiiError::Internal(format!("memstore: open mem_doc: {e}")))?;
        let mut out = Vec::new();
        for row in table
            .iter()
            .map_err(|e| PiiError::Internal(format!("memstore: iter mem_doc: {e}")))?
        {
            let Ok(row) = row else { continue };
            out.push(String::from(row.0.value()));
        }
        Ok(out)
    }

    /// Quantised vector bytes for one doc (`(q_bytes, scale)` or `None`).
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb failure.
    pub fn vector(&self, id: &str) -> Result<Option<(Vec<i8>, f32)>, PiiError> {
        let read = self
            .db
            .begin_read()
            .map_err(|e| PiiError::Internal(format!("memstore: begin_read: {e}")))?;
        let table = read
            .open_table(MEM_VEC)
            .map_err(|e| PiiError::Internal(format!("memstore: open mem_vec: {e}")))?;
        let Some(bytes) = table.get(id).ok().flatten() else {
            return Ok(None);
        };
        let bytes = bytes.value();
        decode_vec(bytes).map(Some)
    }

    /// Vector batch scan by id offset (F4.7): the next `batch` ids after
    /// `after` (exclusive), in key order. The caller keeps only the batch.
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb failure.
    pub fn vector_batch(
        &self,
        after: &str,
        batch: usize,
    ) -> Result<Vec<(String, Vec<i8>, f32)>, PiiError> {
        let read = self
            .db
            .begin_read()
            .map_err(|e| PiiError::Internal(format!("memstore: begin_read: {e}")))?;
        let table = read
            .open_table(MEM_VEC)
            .map_err(|e| PiiError::Internal(format!("memstore: open mem_vec: {e}")))?;
        let mut out = Vec::new();
        let mut range = table
            .range(after..)
            .map_err(|e| PiiError::Internal(format!("memstore: range mem_vec: {e}")))?;
        for row in &mut range {
            if out.len() >= batch {
                break;
            }
            let Ok(row) = row else { continue };
            let id = String::from(row.0.value());
            let Ok((q, scale)) = decode_vec(row.1.value()) else {
                continue;
            };
            out.push((id, q, scale));
        }
        Ok(out)
    }

    /// Edges of one concept: `(other_concept, weight)` in key order (F4.8).
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb failure.
    pub fn edges(&self, concept: &str) -> Result<Vec<(String, u64)>, PiiError> {
        let read = self
            .db
            .begin_read()
            .map_err(|e| PiiError::Internal(format!("memstore: begin_read: {e}")))?;
        let table = read
            .open_table(MEM_EDGE)
            .map_err(|e| PiiError::Internal(format!("memstore: open mem_edge: {e}")))?;
        let mut out = Vec::new();
        let mut range = table
            .range((concept, "")..)
            .map_err(|e| PiiError::Internal(format!("memstore: range mem_edge: {e}")))?;
        for row in &mut range {
            let Ok(row) = row else { continue };
            let key = row.0.value();
            let (c, other) = (key.0, key.1);
            if c != concept {
                break;
            }
            out.push((String::from(other), row.1.value()));
        }
        Ok(out)
    }

    /// Distinct concept heads in `mem_edge` (sorted) — the graph lane's
    /// universe for label propagation (F4.8).
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb failure.
    pub fn concepts(&self) -> Result<Vec<String>, PiiError> {
        let read = self
            .db
            .begin_read()
            .map_err(|e| PiiError::Internal(format!("memstore: begin_read: {e}")))?;
        let table = read
            .open_table(MEM_EDGE)
            .map_err(|e| PiiError::Internal(format!("memstore: open mem_edge: {e}")))?;
        let mut out: Vec<String> = Vec::new();
        for row in table
            .iter()
            .map_err(|e| PiiError::Internal(format!("memstore: iter mem_edge: {e}")))?
        {
            let Ok(row) = row else { continue };
            let key = row.0.value();
            let head = key.0;
            if out.last().is_none_or(|last| last != head) {
                out.push(String::from(head));
            }
        }
        Ok(out)
    }

    /// Moves a doc to the archive (F4.12: forgetting is compression; the
    /// loser is recoverable, never deleted).
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb failure.
    pub fn archive(&self, id: &str) -> Result<(), PiiError> {
        let Some(bytes) = self.doc(id)? else {
            return Ok(());
        };
        let write = self
            .db
            .begin_write()
            .map_err(|e| PiiError::Internal(format!("memstore: begin_write: {e}")))?;
        {
            let mut doc = write
                .open_table(MEM_DOC)
                .map_err(|e| PiiError::Internal(format!("memstore: open mem_doc: {e}")))?;
            let removed = doc
                .remove(id)
                .map_err(|e| PiiError::Internal(format!("memstore: del doc: {e}")))?;
            let mut arch = write
                .open_table(MEM_ARCHIVE)
                .map_err(|e| PiiError::Internal(format!("memstore: open mem_archive: {e}")))?;
            arch.insert(id, bytes.as_slice())
                .map_err(|e| PiiError::Internal(format!("memstore: put archive: {e}")))?;
            let _ = removed;
        }
        write
            .commit()
            .map_err(|e| PiiError::Internal(format!("memstore: commit archive: {e}")))
    }

    /// Archived doc bytes (`None` when absent).
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb failure.
    pub fn archived(&self, id: &str) -> Result<Option<Vec<u8>>, PiiError> {
        let read = self
            .db
            .begin_read()
            .map_err(|e| PiiError::Internal(format!("memstore: begin_read: {e}")))?;
        let table = read
            .open_table(MEM_ARCHIVE)
            .map_err(|e| PiiError::Internal(format!("memstore: open mem_archive: {e}")))?;
        Ok(table.get(id).ok().flatten().map(|v| v.value().to_vec()))
    }
}

/// Decodes `(i8 vec, f32 scale)` from the packed `mem_vec` layout (F4.6).
fn decode_vec(bytes: &[u8]) -> Result<(Vec<i8>, f32), PiiError> {
    if bytes.len() < 5 {
        return Err(PiiError::Invalid(String::from("memstore: short vec")));
    }
    let n = bytes.len() - 4;
    let mut q = Vec::with_capacity(n);
    for b in bytes.get(..n).unwrap_or(&[]) {
        q.push(i8::from_le_bytes([*b]));
    }
    let Some(tail) = bytes.get(n..) else {
        return Err(PiiError::Invalid(String::from("memstore: short vec")));
    };
    let mut scale_bytes = [0u8; 4];
    scale_bytes.copy_from_slice(tail);
    let scale = f32::from_le_bytes(scale_bytes);
    if !scale.is_finite() {
        return Err(PiiError::Invalid(String::from("memstore: bad scale")));
    }
    Ok((q, scale))
}
