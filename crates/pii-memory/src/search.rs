//! Fusion search (F4.9/F4.10): RRF k=60 over the three lanes + temporal
//! decay; honest degradation when the embedder is absent.

use std::sync::Arc;

use pii_abi::PiiError;
use pii_core::Ctx;
use pii_store::mem::MemStore;

use crate::bm25::bm25_search;
use crate::embed::Embedder;
use crate::graph_lane::graph_search;
use crate::rrf::{decayed, rrf};
use crate::vector::vector_search;

/// One fused hit (F4.9).
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryHit {
    /// Doc id.
    pub doc: String,
    /// Fused + decayed score.
    pub score: f32,
    /// Which lanes contributed.
    pub lanes: Vec<String>,
}

/// A full search result (F4.10).
#[derive(Debug, Clone, PartialEq)]
pub struct MemorySearchResult {
    /// Hits in RRF order.
    pub hits: Vec<MemoryHit>,
    /// True when the vector lane was skipped (embedder error / dims 0).
    pub degraded: bool,
    /// The lanes that actually ran.
    pub lanes: Vec<String>,
}

/// The fused searcher over one `MemStore` (F4.9).
pub struct MemorySearcher {
    store: Arc<MemStore>,
    embedder: Option<Arc<dyn Embedder>>,
}

impl MemorySearcher {
    /// Builds with an optional embedder (`None` = degraded vector lane).
    #[must_use]
    pub fn new(store: Arc<MemStore>, embedder: Option<Arc<dyn Embedder>>) -> Self {
        Self { store, embedder }
    }

    /// Runs the fused search (F4.9/F4.10). If the embedder errors or reports
    /// `dims() == 0`, the vector lane is skipped and `degraded` is set —
    /// never silently worse.
    ///
    /// # Errors
    /// Store failures.
    pub fn search(&self, ctx: &Ctx, q: &str, k: usize) -> Result<MemorySearchResult, PiiError> {
        let bm = bm25_search(&self.store, q, k)?;
        let bm_ids: Vec<String> = bm.iter().map(|(d, _)| d.clone()).collect();

        let (mut lanes, mut degraded) = (vec![String::from("bm25")], false);
        let mut lane_lists: Vec<Vec<String>> = vec![bm_ids];

        let vector_ids = match &self.embedder {
            Some(e) if e.dims() > 0 => {
                let vecs = e.embed(&[String::from(q)])?;
                lanes.push(String::from("vector"));
                let v = vector_search(&self.store, vecs.first().map_or(&[], Vec::as_slice), k)?;
                let ids: Vec<String> = v.iter().map(|(d, _)| d.clone()).collect();
                lane_lists.push(ids);
                Some(v)
            }
            _ => {
                degraded = true;
                None
            }
        };

        let graph = graph_search(&self.store, q, k)?;
        lanes.push(String::from("graph"));
        lane_lists.push(graph.iter().map(|(d, _)| d.clone()).collect());

        let fused = rrf(&lane_lists);
        let now = ctx.now_ms();
        let mut hits = Vec::with_capacity(fused.len());
        for (doc, score) in fused.into_iter().take(k) {
            let kind = self.kind_of(&doc);
            let created = self.created_of(&doc);
            #[allow(clippy::cast_precision_loss)] // reason: age in days
            let age_days = now.saturating_sub(created) as f32 / 86_400_000.0;
            let mut lns: Vec<String> = Vec::new();
            if bm.iter().any(|(d, _)| d == &doc) {
                lns.push(String::from("bm25"));
            }
            if let Some(v) = &vector_ids
                && v.iter().any(|(d, _)| d == &doc)
            {
                lns.push(String::from("vector"));
            }
            if graph.iter().any(|(d, _)| d == &doc) {
                lns.push(String::from("graph"));
            }
            hits.push(MemoryHit {
                doc,
                score: decayed(score, kind, age_days),
                lanes: lns,
            });
        }
        if degraded {
            lanes = vec![String::from("bm25"), String::from("graph")];
        }
        Ok(MemorySearchResult {
            hits,
            degraded,
            lanes,
        })
    }

    fn kind_of(&self, id: &str) -> crate::doc::DocKind {
        crate::doc::decode_kind(self.store.doc(id).ok().flatten())
    }

    fn created_of(&self, id: &str) -> u64 {
        crate::doc::created_of(self.store.doc(id).ok().flatten())
    }
}
