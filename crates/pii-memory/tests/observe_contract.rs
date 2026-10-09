//! Spec 100 R3: search and consolidate outcomes reach a `pii-observe`
//! observer by their registered names; the searcher reports the store's health.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::Arc;

use pii_core::{Budget, CapKind, CapSet, Ctx};
use pii_memory::consolidate::ConsolidateCfg;
use pii_memory::{MemorySearcher, NullEmbedder, consolidate_observed};
use pii_observe::{Health, Recorder, names};
use pii_store::MemStore;

fn ctx() -> Ctx {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::SPAWN;
    Ctx::root(caps, Budget::unlimited())
}

fn store() -> (tempfile::TempDir, Arc<MemStore>) {
    let root = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp"));
    std::fs::create_dir_all(&root).unwrap();
    let dir = tempfile::Builder::new()
        .prefix("mem-observe-")
        .tempdir_in(root)
        .unwrap();
    let s = Arc::new(MemStore::open(&dir.path().join("m.redb")).unwrap());
    (dir, s)
}

#[test]
fn search_outcome_reaches_the_observer_by_name() {
    let (_d, store) = store();
    let rec = Arc::new(Recorder::new());
    let searcher =
        MemorySearcher::new(store, Some(Arc::new(NullEmbedder))).with_observer(rec.clone());
    searcher.search(&ctx(), "anything", 3).unwrap();
    let ev = rec.events_named(names::MEMORY_SEARCH);
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0].fields.get("outcome"), Some(&"ok".into()));
    assert_eq!(ev[0].fields.get("hits"), Some(&0.into()));
    let h = searcher.health();
    assert_eq!(h.name, "memory");
    assert_eq!(h.identity.get("docs").map(String::as_str), Some("0"));
}

#[test]
fn consolidate_outcome_reaches_the_observer_by_name() {
    let (_d, store) = store();
    let rec = Recorder::new();
    let cfg = ConsolidateCfg {
        archive_after_days: 10_000.0,
        same_layer_only: false,
    };
    consolidate_observed(&store, &ctx(), &cfg, &rec).unwrap();
    let ev = rec.events_named(names::MEMORY_CONSOLIDATE);
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0].fields.get("merged"), Some(&0.into()));
}
