//! Graph lane (F4.8): concept co-occurrence expansion + label propagation
//! communities. Zero model calls anywhere.
//!
//! **When it fires** (spec 024 N4, closes C5): [`graph_search`] contributes
//! hits only when a query token is ALSO a concept head already present in
//! `mem_edge` — i.e. some prior doc was written via `write_fact`/`memory.put`
//! with that word in its `concepts` list. A query with no concept overlap
//! (the common case for free-text queries against docs stored without
//! explicit concepts) makes `query_concepts` return empty and the lane
//! contributes nothing, even though `lanes` in the response always lists
//! `"graph"` as a lane that RAN, not one that HIT. See
//! `pii-memory/tests/lanes.rs::graph_lane_recalls_doc_via_shared_concept`
//! for a positive, end-to-end demonstration.

use pii_abi::PiiError;
use pii_store::mem::MemStore;

use crate::tokenize::tokenize;

/// Expansion breadth (F4.8: top 8 co-occurring concepts).
pub const EXPAND_TOP: usize = 8;

/// Query concepts: the query's tokens that exist as concepts (F4.8).
///
/// # Errors
/// [`PiiError::Internal`] on a store failure.
pub fn query_concepts(store: &MemStore, q: &str) -> Result<Vec<String>, PiiError> {
    let universe = store.concepts()?;
    Ok(tokenize(q)
        .into_iter()
        .filter(|t| universe.iter().any(|c| c == t))
        .collect())
}

/// Expands each query concept to its top-8 co-occurring concepts and scores
/// documents by summed edge weight (F4.8).
///
/// # Errors
/// [`PiiError::Internal`] on a store failure.
pub fn graph_search(store: &MemStore, q: &str, k: usize) -> Result<Vec<(String, f32)>, PiiError> {
    let concepts = query_concepts(store, q)?;
    if concepts.is_empty() {
        return Ok(Vec::new());
    }
    // Expand: concept -> top co-occurring heads (F4.8).
    // mem_edge is (concept_head, other) — a doc-level co-occurrence edge.
    let mut acc: Vec<(String, f32)> = Vec::new();
    for c in &concepts {
        let edges = store.edges(c)?;
        let mut sorted = edges.clone();
        sorted.sort_by_key(|(_, w)| std::cmp::Reverse(*w));
        for (other, w) in sorted.into_iter().take(EXPAND_TOP) {
            #[allow(clippy::cast_precision_loss)] // reason: edge weights, f32 scores
            let w_f = w as f32;
            if let Some(e) = acc.iter_mut().find(|(d, _)| d == &other) {
                e.1 += w_f;
            } else {
                acc.push((other, w_f));
            }
        }
    }
    acc.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    Ok(acc.into_iter().take(k).collect())
}

/// Label propagation over `mem_edge` (F4.8): concepts in sorted order,
/// 10 rounds, ties broken lexicographically. Returns `concept -> community`.
///
/// # Errors
/// [`PiiError::Internal`] on a store failure.
pub fn label_propagation(
    store: &MemStore,
) -> Result<std::collections::BTreeMap<String, u32>, PiiError> {
    let heads = store.concepts()?;
    let mut label: std::collections::BTreeMap<String, u32> = heads
        .iter()
        .enumerate()
        .map(|(i, c)| (c.clone(), u32::try_from(i).unwrap_or(u32::MAX)))
        .collect();
    for _ in 0..10 {
        let mut next = label.clone();
        for c in &heads {
            let edges = store.edges(c)?;
            // Majority vote over neighbours, ties → lexicographically
            // smallest label (deterministic, F4.8).
            let mut votes: std::collections::BTreeMap<u32, u64> = std::collections::BTreeMap::new();
            for (other, w) in edges {
                if let Some(l) = label.get(&other) {
                    *votes.entry(*l).or_insert(0) += w;
                }
            }
            if let Some((best, _)) = votes
                .iter()
                .max_by_key(|(l, w)| (*w, std::cmp::Reverse(*l)))
            {
                next.insert(c.clone(), *best);
            }
        }
        label = next;
    }
    Ok(label)
}
