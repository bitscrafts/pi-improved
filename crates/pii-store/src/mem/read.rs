//! Read APIs for the memory lanes (F4.5/F4.6/F4.7/F4.8). All reads stream
//! from redb — no in-RAM inverted index, no full-corpus resident array.

use pii_abi::PiiError;
use redb::{ReadableDatabase, ReadableTable};

use crate::mem::{MemStore, Posting};
use crate::tables::{MEM_ARCHIVE, MEM_DF, MEM_DOC, MEM_EDGE, MEM_LAYER, MEM_TERM, MEM_VEC};

impl MemStore {
    /// Stored layer tag for one doc (`None` when absent or never written —
    /// pre-existing docs carry no `mem_layer` row).
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb failure.
    pub fn layer(&self, id: &str) -> Result<Option<u8>, PiiError> {
        let read = self
            .db
            .begin_read()
            .map_err(|e| PiiError::Internal(format!("memstore: begin_read: {e}")))?;
        let table = read
            .open_table(MEM_LAYER)
            .map_err(|e| PiiError::Internal(format!("memstore: open mem_layer: {e}")))?;
        Ok(table.get(id).ok().flatten().map(|v| v.value()))
    }
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
