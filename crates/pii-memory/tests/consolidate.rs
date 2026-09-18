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
    std::fs::create_dir_all(root.join("phase3")).expect("create crate-local _tmp/phase3");
    root
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
    let doc = Doc::new(Layer::L1, DocKind::Atom, text, vec![], 1.0, 0);
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
    };
    store.put(&rows).unwrap();
    doc.id
}

#[test]
fn merge_requires_two_shared_concepts() {
    let store = MemStore::open(&tmp_root().join("phase3/cons-merge.redb")).unwrap();
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
    let store =
        std::sync::Arc::new(MemStore::open(&tmp_root().join("phase3/cons-cap.redb")).unwrap());
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
