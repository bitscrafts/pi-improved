//! Vector lane (F4.7): brute-force scan streaming from `mem_vec` in batches
//! of 512, holding one batch + a top-k heap. No full-corpus resident array.

use pii_abi::PiiError;
use pii_store::mem::MemStore;

use crate::quant::{cosine_q, quantise};

/// Batch size (F4.7).
pub const BATCH: usize = 512;

/// One vector-lane hit.
#[derive(Debug, Clone, PartialEq)]
pub struct VecHit {
    /// Doc id.
    pub doc: String,
    /// Cosine similarity.
    pub score: f32,
}

/// Scans the corpus in batches against the embedded query (F4.7).
///
/// # Errors
/// [`PiiError::Internal`] on a store failure.
pub fn vector_search(
    store: &MemStore,
    qvec: &[f32],
    k: usize,
) -> Result<Vec<(String, f32)>, PiiError> {
    let (query_q, qscale) = quantise(qvec);
    let mut heap: Vec<(String, f32)> = Vec::new();
    let mut after = String::new();
    loop {
        let batch = store.vector_batch(&after, BATCH)?;
        if batch.is_empty() {
            break;
        }
        let n = batch.len();
        let last = batch.last().map(|(id, _, _)| id.clone());
        for (doc_id, stored, scale) in batch {
            let sim = cosine_q(&query_q, qscale, &stored, scale);
            heap.push((doc_id, sim));
        }
        heap.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        heap.truncate(k);
        match last {
            Some(l) => after = l,
            None => break,
        }
        if n < BATCH {
            break;
        }
    }
    Ok(heap.into_iter().take(k).collect())
}
