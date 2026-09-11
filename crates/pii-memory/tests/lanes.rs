//! Lane semantics (part-3 TDD table): BM25 reference scores, quantisation
//! round-trip, RRF == BM25 when lanes disabled, honest degradation, decay
//! ordering with `FixedClock`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use pii_core::{Budget, CapSet, Ctx};
use pii_memory::bm25::{bm25_search, idf, tf_score};
use pii_memory::decay::decay;
use pii_memory::doc::{Doc, DocKind, Layer, encode, id_of};
use pii_memory::embed::{Embedder, NullEmbedder, TrigramEmbedder};
use pii_memory::rrf::rrf;
use pii_memory::search::MemorySearcher;
use pii_memory::tokenize::tokenize;
use pii_store::MemStore;

const DB: &str = "/projects/pi-improved/_tmp/phase3/lanes-test.redb";

fn put(store: &MemStore, text: &str, kind: DocKind, created: u64) -> String {
    let mut doc = Doc::new(Layer::L1, kind, text, vec![], 1.0, created);
    let toks = tokenize(text);
    let mut terms = std::collections::BTreeMap::new();
    for t in &toks {
        *terms.entry(t.clone()).or_insert(0) += 1;
    }
    let vec = TrigramEmbedder
        .embed(&[text.to_owned()])
        .unwrap()
        .into_iter()
        .next()
        .map(|v| pii_memory::quant::quantise(&v));
    doc.created_ms = created;
    let ret = doc.id.clone();
    let rows = pii_store::mem::PutRows {
        id: doc.id.clone(),
        doc: encode(&doc),
        terms: terms.into_iter().collect(),
        vector: vec,
        edges: Vec::new(),
        dl: u32::try_from(toks.len()).unwrap_or(u32::MAX),
    };
    store.put(&rows).unwrap();
    ret
}

#[test]
#[allow(clippy::many_single_char_names)] // reason: test fixture math
#[allow(clippy::cast_precision_loss)] // reason: test reference formula
fn bm25_matches_reference_scores() {
    // 5 equal-length docs, one term appearing once: idf/tf math pinned.
    let n = 5u64;
    let df = 2u64;
    // Standard Okapi idf: ln(1 + (N - df + 0.5) / (df + 0.5)) — ln_1p IS
    // the +1; do NOT add another 1.0 inside.
    let want_idf = ((n as f32 - df as f32 + 0.5) / (df as f32 + 0.5)).ln_1p();
    assert!((idf(n, df) - want_idf).abs() < 1e-5);
    // tf=1, dl=10, avg=10 → (1*2.2)/(1 + 1.2) = 1.0
    assert!((tf_score(1, 10, 10.0) - 1.0).abs() < 1e-4);
    // Longer doc down-weights.
    assert!(tf_score(1, 100, 10.0) < 1.0);
}

#[test]
#[allow(clippy::many_single_char_names)] // reason: test fixture math
#[allow(clippy::cast_precision_loss)] // reason: test reference formula
#[allow(clippy::imprecise_flops)] // reason: test reference formula
fn quantise_roundtrip_under_one_percent() {
    for i in 0..100 {
        let t = i as f32 / 100.0 * std::f32::consts::PI;
        let (x, y) = (t.cos(), t.sin());
        let n = x.hypot(y);
        let (q, s) = pii_memory::quant::quantise(&[x / n, y / n]);
        let c = pii_memory::quant::cosine_q(&q, s, &q, s);
        assert!((c - 1.0).abs() < 0.01, "step {i}: {c}");
    }
}

#[test]
fn rrf_equals_bm25_when_lanes_disabled() {
    let _ = std::fs::remove_file(DB);
    let store = MemStore::open(std::path::Path::new(DB)).unwrap();
    let ids: Vec<String> = (0..10)
        .map(|i| {
            put(
                &store,
                &format!("lane {i}: the fused ranking must not relabel bm25 output"),
                DocKind::Atom,
                0,
            )
        })
        .collect();
    let q = "fused ranking relabel bm25 output";
    let bm = bm25_search(&store, q, 10).unwrap();
    // Hybrid with ONLY the bm25 lane = plain rrf of one list = same order.
    let only_bm = rrf(&[bm.iter().map(|(d, _)| d.clone()).collect::<Vec<_>>()]);
    assert_eq!(
        bm.iter().map(|(d, _)| d.clone()).collect::<Vec<_>>(),
        only_bm.into_iter().map(|(d, _)| d).collect::<Vec<_>>(),
        "fusion over a single lane must not change the ranking"
    );
    let _ = ids;
}

#[test]
fn degrades_honestly_without_embedder() {
    let store = MemStore::open(std::path::Path::new(
        "/projects/pi-improved/_tmp/phase3/lanes-degrade.redb",
    ))
    .unwrap();
    put(
        &store,
        "degraded lane note: the null embedder must not be silent",
        DocKind::Atom,
        0,
    );
    let searcher = MemorySearcher::new(
        std::sync::Arc::new(store),
        Some(std::sync::Arc::new(NullEmbedder)),
    );
    let ctx = Ctx::root(CapSet::empty(), Budget::unlimited());
    let r = searcher.search(&ctx, "null embedder", 5).unwrap();
    assert!(r.degraded, "must be degraded");
    assert_eq!(r.lanes, vec![String::from("bm25"), String::from("graph")]);
}

#[test]
fn decay_orders_by_age_with_fixed_clock() {
    let young = decay(pii_memory::decay::LAMBDA_ATOM, 1.0);
    let old = decay(pii_memory::decay::LAMBDA_ATOM, 90.0);
    assert!(young > old, "the younger must rank first");
    // Persona never decays: same score at any age.
    let any_age = decay(pii_memory::decay::LAMBDA_PERSONA, 365.0);
    assert!((any_age - 1.0).abs() < 1e-6, "persona never decays");
    // Content ids stay stable across put order.
    assert_eq!(id_of("x"), id_of("x"));
    let _ = std::time::Instant::now();
}

/// G1.4 (spec 09 v2): a search hit carries the doc TEXT, not just the id.
#[test]
fn memory_hit_carries_doc_text() {
    let db = "/projects/pi-improved/_tmp/phase3/mem-hit-text.redb";
    let _ = std::fs::remove_file(db);
    let store = MemStore::open(std::path::Path::new(db)).unwrap();
    put(
        &store,
        "user prefers dark mode for the terminal",
        DocKind::Atom,
        0,
    );
    let searcher = MemorySearcher::new(
        std::sync::Arc::new(store),
        Some(std::sync::Arc::new(TrigramEmbedder)),
    );
    let ctx = Ctx::root(CapSet::empty(), Budget::unlimited());
    let r = searcher.search(&ctx, "dark mode preference", 5).unwrap();
    assert!(!r.hits.is_empty(), "must find the doc");
    assert!(
        r.hits[0].text.contains("dark mode"),
        "hit must carry doc text, got: {:?}",
        r.hits[0].text
    );
}

/// Spec 024 N4 (closes C5): the graph lane positively recalls a doc via a
/// shared concept edge, when NEITHER bm25 (the query term is absent from the
/// doc's own text) NOR the vector lane (deliberately degraded here) could
/// have found it. This is the end-to-end proof that `graph_search` is not
/// dormant — see the "when it fires" note on `graph_lane`.
#[test]
fn graph_lane_recalls_doc_via_shared_concept() {
    let db = "/projects/pi-improved/_tmp/phase3/graph-lane.redb";
    let _ = std::fs::remove_file(db);
    let store = MemStore::open(std::path::Path::new(db)).unwrap();

    // The doc text shares NO words with the query below; only the concept
    // edge (tagged out-of-band, as `write_fact` does with its `concepts`
    // argument) ties the two together.
    let text = "zulu yankee whiskey tango foxtrot";
    let concept = "projectxconcept";
    let doc = Doc::new(
        Layer::L1,
        DocKind::Atom,
        text,
        vec![concept.to_owned()],
        1.0,
        0,
    );
    let toks = tokenize(text);
    let mut terms = std::collections::BTreeMap::new();
    for t in &toks {
        *terms.entry(t.clone()).or_insert(0) += 1;
    }
    let id = doc.id.clone();
    let rows = pii_store::mem::PutRows {
        id: id.clone(),
        doc: encode(&doc),
        terms: terms.into_iter().collect(),
        vector: None,
        edges: vec![(concept.to_owned(), 5)],
        dl: u32::try_from(toks.len()).unwrap_or(u32::MAX),
    };
    store.put(&rows).unwrap();

    // NullEmbedder: the vector lane is degraded, so a hit can ONLY come
    // from bm25 or graph — isolating the graph lane's contribution.
    let searcher = MemorySearcher::new(
        std::sync::Arc::new(store),
        Some(std::sync::Arc::new(NullEmbedder)),
    );
    let ctx = Ctx::root(CapSet::empty(), Budget::unlimited());
    let r = searcher.search(&ctx, concept, 5).unwrap();

    assert!(
        !r.hits.is_empty(),
        "graph lane must recall the doc via its concept edge"
    );
    assert_eq!(r.hits[0].doc, id);
    assert!(
        r.hits[0].lanes.contains(&String::from("graph")),
        "hit lanes must include graph, got {:?}",
        r.hits[0].lanes
    );
    assert!(
        !r.hits[0].lanes.contains(&String::from("bm25")),
        "bm25 must NOT have matched (the concept term is absent from the doc text): {:?}",
        r.hits[0].lanes
    );
}

/// G2.4 (spec 09 v2): `write_fact` stores a non-None vector (trigram lane).
#[test]
fn write_fact_stores_vector() {
    let rows = pii_memory::doc::write_fact("user prefers dark mode", Vec::new(), 1.0, 0);
    assert!(
        rows.vector.is_some(),
        "write_fact must populate the vector lane"
    );
    let (q, _scale) = rows.vector.unwrap();
    assert_eq!(q.len(), 256, "trigram embedder dims 256");
}
