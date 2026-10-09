//! The observability seam of `pii-memory`: search and consolidate outcomes as
//! `pii-observe` events, and the store's [`Health`] surfaced on the searcher.

use std::sync::Arc;

use pii_abi::PiiError;
use pii_core::Ctx;
use pii_observe::{Event, Health, Level, Observable, Observed, Observer, SubsystemHealth, names};
use pii_store::MemStore;

use crate::consolidate::{ConsolidateCfg, ConsolidateReport, consolidate};
use crate::search::{MemorySearchResult, MemorySearcher};

impl Observable for MemorySearcher {
    fn observer(&self) -> Option<&dyn Observer> {
        self.observer.as_deref()
    }
}

impl MemorySearcher {
    /// Attaches the observer that [`MemorySearcher::search`] reports to.
    ///
    /// A builder rather than a `new` argument: `new` has several call sites
    /// that have no observer yet.
    #[must_use]
    pub fn with_observer(mut self, observer: Arc<dyn Observer>) -> Self {
        self.observer = Some(observer);
        self
    }

    /// Reports a search outcome: hit count, degradation and lanes — never the
    /// query text or the hit text.
    pub(crate) fn observe_search(&self, result: &Result<MemorySearchResult, PiiError>) {
        let ev = match result {
            Ok(r) => Event::new("memory", Level::Info, names::MEMORY_SEARCH)
                .field("outcome", "ok")
                .field("hits", r.hits.len() as u64)
                .field("degraded", r.degraded)
                .field("lanes", r.lanes.join(",")),
            Err(e) => Event::new("memory", Level::Error, names::MEMORY_SEARCH)
                .field("outcome", "error")
                .field("error", e.to_string()),
        };
        self.emit(&ev);
    }
}

impl Health for MemorySearcher {
    fn health(&self) -> SubsystemHealth {
        self.store_health()
    }
}

/// [`consolidate`], reporting the outcome to `observer` as a
/// [`names::MEMORY_CONSOLIDATE`] event. The result is returned unchanged.
///
/// # Errors
/// As [`consolidate`].
pub fn consolidate_observed(
    store: &Arc<MemStore>,
    ctx: &Ctx,
    cfg: &ConsolidateCfg,
    observer: &dyn Observer,
) -> Result<ConsolidateReport, PiiError> {
    let result = consolidate(store, ctx, cfg);
    let ev = match &result {
        Ok(r) => Event::new("memory", Level::Info, names::MEMORY_CONSOLIDATE)
            .field("outcome", "ok")
            .field("candidates", u64::from(r.candidates_evaluated))
            .field("merged", u64::from(r.merged))
            .field("archived", u64::from(r.archived)),
        Err(e) => Event::new("memory", Level::Error, names::MEMORY_CONSOLIDATE)
            .field("outcome", "error")
            .field("error", e.to_string()),
    };
    observer.event(&ev);
    result
}
