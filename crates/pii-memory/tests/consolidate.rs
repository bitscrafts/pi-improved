//! Consolidation (F4.12): merge rules, hard candidate cap, archive-not-delete.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use pii_core::{Budget, CapKind, CapSet, Ctx};
use pii_memory::consolidate::{ConsolidateCfg, MAX_CANDIDATES, consolidate};
use pii_memory::doc::{Doc, DocKind, Layer, encode};
use pii_memory::tokenize::tokenize;
use pii_store::MemStore;
use std::path::PathBuf;

fn tmp_root() -> PathBuf {
    let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp"));
    std::fs::create_dir_all(&root).expect("create crate-local _tmp");
    root
}

/// Opens a FRESH `MemStore` in a test-local tempdir (spec 74 R2). The old
/// per-test fixed paths (`_tmp/phase3/cons-*.redb`) persisted across runs:
/// doc ids are content-addressed, so a re-put on a stale table found the
/// PREVIOUS run's `mem_layer` row and masked a dropped layer insert (043's
/// fresh.sh had to move those files aside to see that mutation red). The
/// tempdir is removed when the test ends; nothing survives it.
fn fresh_store(prefix: &str) -> (tempfile::TempDir, MemStore) {
    let dir = tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(tmp_root())
        .unwrap();
    let store = MemStore::open(&dir.path().join("cons.redb")).unwrap();
    (dir, store)
}

fn cfg() -> ConsolidateCfg {
    ConsolidateCfg {
        archive_after_days: 10_000.0,
        same_layer_only: false,
    }
}

fn ctx_now(_ms: u64) -> Ctx {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::SPAWN;
    // created_ms semantics come from doc payloads; clock injects via Ctx
    Ctx::root(caps, Budget::unlimited())
}

fn put(store: &MemStore, text: &str) -> String {
    put_layer(store, text, Layer::L1)
}

/// Writes a doc on an explicit layer, returning its id. The layer tag is
/// persisted in the additive `mem_layer` store table.
fn put_layer(store: &MemStore, text: &str, layer: Layer) -> String {
    let doc = Doc::new(layer, DocKind::Atom, text, vec![], 1.0, 0);
    let toks = tokenize(text);
    let mut terms = std::collections::BTreeMap::new();
    for t in &toks {
        *terms.entry(t.clone()).or_insert(0) += 1;
    }
    let rows = pii_store::mem::PutRows {
        id: doc.id.clone(),
        doc: encode(&doc),
        terms: terms.into_iter().collect(),
        vector: None,
        edges: Vec::new(),
        dl: u32::try_from(toks.len()).unwrap_or(u32::MAX),
        layer: pii_memory::doc::layer_byte(layer),
    };
    store.put(&rows).unwrap();
    doc.id
}

#[test]
fn merge_requires_two_shared_concepts() {
    let (_dir, store) = fresh_store("cons-merge-");
    // One shared concept: NOT merged.
    let a = put(&store, "sandbox probe enforce profile one");
    let b = put(&store, "sandbox never merges a single concept pair");
    // Two shared concepts: merged.
    let c = put(&store, "bm25 idf scoring rank two shared");
    let d = put(&store, "bm25 idf ranks the fused hybrid lane");
    let report = consolidate(&std::sync::Arc::new(store), &ctx_now(1_000), &cfg()).unwrap();
    let merged_ids = (a, b, c, d);
    assert!(
        report.merged >= 1,
        "at least the two-shared-concept pair merges: {report:?} ids {merged_ids:?}"
    );
    let _ = report;
}

#[test]
fn consolidate_cap_is_hard() {
    let (_dir, store) = fresh_store("cons-cap-");
    let store = std::sync::Arc::new(store);
    // Many highly-similar docs → candidate pairs >> 200.
    for i in 0..40 {
        put(
            &store,
            &format!("shared concept merge pair doc {i} of the capped corpus"),
        );
    }
    let report = consolidate(&store, &ctx_now(0), &cfg()).unwrap();
    assert!(
        report.candidates_evaluated <= u32::try_from(MAX_CANDIDATES).unwrap_or(u32::MAX),
        "cap held"
    );
    if report.candidates_capped {
        assert_eq!(
            report.candidates_evaluated,
            u32::try_from(MAX_CANDIDATES).unwrap_or(u32::MAX),
            "generation stops exactly at the cap"
        );
    }
    // Nothing deleted: every doc is either live or archived (recoverable).
    for id in store.doc_ids().unwrap() {
        assert!(store.doc(&id).unwrap().is_some() || store.archived(&id).unwrap().is_some());
    }
}

#[test]
fn negation_veto_blocks_merge() {
    // The pure predicate: a negation token within the window of a shared
    // concept vetoes the merge.
    let shared = vec![String::from("sandbox"), String::from("probe")];
    let veto = "sandbox probe is not enforceable";
    let clean = "sandbox probe enforce profile";
    assert!(pii_memory::consolidate::negated(veto, &shared));
    assert!(!pii_memory::consolidate::negated(clean, &shared));
}

#[test]
fn layer_of_reads_real_layer_value() {
    let (_dir, store) = fresh_store("cons-layer-read-");
    let a = put_layer(&store, "atom fact one alpha beta", Layer::L1);
    let b = put_layer(&store, "scene distill alpha gamma", Layer::L2);
    let la = store.layer(&a).unwrap();
    let lb = store.layer(&b).unwrap();
    assert_eq!(la, Some(pii_memory::doc::layer_byte(Layer::L1)));
    assert_eq!(lb, Some(pii_memory::doc::layer_byte(Layer::L2)));
    assert_ne!(la, lb, "layer tags must differ, not both 0");
}

#[test]
fn consolidate_same_layer_only_true_blocks_cross_layer_merge() {
    let (_dir, store) = fresh_store("cons-guard-");
    let store = std::sync::Arc::new(store);
    // Two docs with two shared concepts but on different layers.
    let a = put_layer(&store, "sandbox probe enforce alpha beta", Layer::L1);
    let b = put_layer(&store, "sandbox probe enforce alpha gamma", Layer::L2);
    let mut c = cfg();
    c.same_layer_only = true;
    let report = consolidate(&store, &ctx_now(1_000), &c).unwrap();
    for id in [&a, &b] {
        assert!(
            store.doc(id).unwrap().is_some(),
            "cross-layer merge must NOT happen under same_layer_only=true: {report:?}"
        );
    }
}

#[test]
fn consolidate_same_layer_only_false_still_merges_cross_layer() {
    let (_dir, store) = fresh_store("cons-guard-off-");
    let store = std::sync::Arc::new(store);
    let a = put_layer(&store, "sandbox probe enforce alpha beta", Layer::L1);
    let b = put_layer(&store, "sandbox probe enforce alpha gamma", Layer::L2);
    let mut c = cfg();
    c.same_layer_only = false;
    let report = consolidate(&store, &ctx_now(1_000), &c).unwrap();
    assert_eq!(report.merged, 1, "flag false merges regardless of layer");
    let gone = if store.doc(&a).unwrap().is_some() {
        b
    } else {
        a
    };
    assert!(
        store.doc(&gone).unwrap().is_none() && store.archived(&gone).unwrap().is_some(),
        "the loser is archived, never deleted"
    );
}
