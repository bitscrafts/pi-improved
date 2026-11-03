//! Write APIs for the memory store (F4.11/F4.12): the single-txn put, its
//! idempotence probe, and the df bump.

use pii_abi::PiiError;
use redb::{ReadableDatabase, ReadableTable};

use crate::mem::{MemStore, PutRows};
use crate::tables::{MEM_DF, MEM_DOC, MEM_EDGE, MEM_EVIDENCE, MEM_LAYER, MEM_TERM, MEM_VEC};

impl MemStore {
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
            let mut layer = write
                .open_table(MEM_LAYER)
                .map_err(|e| PiiError::Internal(format!("memstore: open mem_layer: {e}")))?;
            layer
                .insert(rows.id.as_str(), rows.layer)
                .map_err(|e| PiiError::Internal(format!("memstore: put layer: {e}")))?;
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
        // df: one per distinct term, counted once per genuinely new doc.
        if existed {
            return Ok(());
        }
        self.bump_df(&rows.id)
    }

    /// Records the evidence of one doc (spec 210): `(reference, ms)`, replacing any earlier record.
    /// The doc's own bytes are not touched.
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb failure.
    pub fn set_evidence(&self, id: &str, reference: &str, ms: u64) -> Result<(), PiiError> {
        let mut bytes = ms.to_le_bytes().to_vec();
        bytes.extend_from_slice(reference.as_bytes());
        let write = self
            .db
            .begin_write()
            .map_err(|e| PiiError::Internal(format!("memstore: begin_write: {e}")))?;
        {
            let mut table = write
                .open_table(MEM_EVIDENCE)
                .map_err(|e| PiiError::Internal(format!("memstore: open mem_evidence: {e}")))?;
            table
                .insert(id, bytes.as_slice())
                .map_err(|e| PiiError::Internal(format!("memstore: put evidence: {e}")))?;
        }
        write
            .commit()
            .map_err(|e| PiiError::Internal(format!("memstore: commit evidence: {e}")))
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    /// Spec 74 R2: a doc put with a `mem_layer` value reads its layer back —
    /// the additive table is written in the same single txn as the doc.
    /// Hermetic: the store lives in a test-local tempdir; a stale persisted
    /// `.redb` previously masked exactly this insert when it was dropped,
    /// because content-addressed ids hit a prior run's layer row.
    #[test]
    fn put_persists_mem_layer_and_reads_back() {
        let root = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp"));
        std::fs::create_dir_all(&root).unwrap();
        let dir = tempfile::Builder::new()
            .prefix("mem-layer-")
            .tempdir_in(&root)
            .unwrap();
        let store = MemStore::open(&dir.path().join("mem-layer.redb")).unwrap();
        let rows = PutRows {
            id: String::from("mem-layer-test-doc"),
            doc: vec![1, 2, 3],
            terms: Vec::new(),
            vector: None,
            edges: Vec::new(),
            dl: 0,
            layer: 2,
        };
        store.put(&rows).unwrap();
        assert_eq!(store.layer("mem-layer-test-doc").unwrap(), Some(2));
    }
}
