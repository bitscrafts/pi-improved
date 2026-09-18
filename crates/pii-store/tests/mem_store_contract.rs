//! Contract tests for `MemStore` directly (F4.2/F4.5–F4.12).
//!
//! Each test owns its database under `_tmp/` (CARGO_MANIFEST_DIR-relative,
//! per F-15 — never a hardcoded absolute checkout path).
//! Test-only addition: nothing under `pii-store/src/` is modified.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::PathBuf;

use pii_store::{MemStore, mem::PutRows};

fn tmp_path(name: &str) -> PathBuf {
    let dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../_tmp"))
        .join("mem-store-test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!(
        "{}-{}-{}.redb",
        name,
        std::process::id(),
        std::time::Instant::now().elapsed().as_nanos()
    ));
    let _ = std::fs::remove_file(&path);
    path
}

fn put_rows(id: &str, dl: u32) -> PutRows {
    PutRows {
        id: id.to_string(),
        doc: format!("doc-bytes-{id}").into_bytes(),
        terms: Vec::new(),
        vector: None,
        edges: Vec::new(),
        dl,
        layer: 0,
    }
}

#[test]
fn put_then_read_doc_roundtrips() {
    let path = tmp_path("put-read-roundtrip");
    let store = MemStore::open(&path).unwrap();
    let rows = put_rows("a1", 7);
    store.put(&rows).unwrap();

    assert_eq!(
        store.doc("a1").unwrap().as_deref(),
        Some(rows.doc.as_slice())
    );
    assert!(store.doc("missing").unwrap().is_none());
    assert_eq!(store.doc_ids().unwrap(), vec!["a1".to_string()]);
    assert_eq!(store.n_docs().unwrap(), 1);
    assert_eq!(store.sum_dl().unwrap(), 7);
}

#[test]
fn put_is_idempotent_on_id() {
    let path = tmp_path("put-idempotent");
    let store = MemStore::open(&path).unwrap();
    let mut first = put_rows("a1", 7);
    first.terms = vec![("alpha".to_string(), 3)];

    let mut second = put_rows("a1", 12);
    second.terms = vec![("beta".to_string(), 1), ("gamma".to_string(), 2)];

    store.put(&first).unwrap();
    store.put(&second).unwrap();

    // Second payload wins.
    assert_eq!(
        store.doc("a1").unwrap().as_deref(),
        Some(second.doc.as_slice())
    );
    // Doc count stays 1. NOTE (observed behavior, G1): put() only bumps
    // n_docs/sum_dl when the doc did not exist, so an idempotent re-put
    // leaves sum_dl at the FIRST payload's dl (7), not the second's. This
    // looks like a real bug candidate (sum_dl goes stale on re-put) but is
    // recorded here as-is per the guardrail.
    assert_eq!(store.n_docs().unwrap(), 1);
    assert_eq!(store.sum_dl().unwrap(), 7);
    // Old term rows: redb upserts, so the replaced doc's term rows persist
    // unless the implementation deletes them — assert what put() actually
    // does: the new doc's terms are readable and n_docs stays 1.
    assert_eq!(store.postings("beta").unwrap().len(), 1);
    assert_eq!(store.postings("gamma").unwrap().len(), 1);
}

#[test]
fn postings_and_df_reflect_puts() {
    let path = tmp_path("postings-df");
    let store = MemStore::open(&path).unwrap();

    let mut a = put_rows("a1", 3);
    a.terms = vec![("shared".to_string(), 2), ("only_a".to_string(), 1)];
    let mut b = put_rows("b2", 5);
    b.terms = vec![("shared".to_string(), 4)];
    let mut c = put_rows("c3", 2);
    c.terms = vec![("shared".to_string(), 1), ("only_c".to_string(), 1)];

    store.put(&a).unwrap();
    store.put(&b).unwrap();
    store.put(&c).unwrap();

    let shared = store.postings("shared").unwrap();
    assert_eq!(shared.len(), 3, "three docs share 'shared'");
    assert!(shared.iter().all(|p| p.doc != "zz"));
    for (id, tf) in [("a1", 2u32), ("b2", 4), ("c3", 1)] {
        let p = shared
            .iter()
            .find(|p| p.doc == id)
            .unwrap_or_else(|| panic!("missing {id}"));
        assert_eq!(p.tf, tf);
    }
    // Key order: sorted by doc id within a term.
    let docs: Vec<&str> = shared.iter().map(|p| p.doc.as_str()).collect();
    assert_eq!(docs, vec!["a1", "b2", "c3"]);

    assert!(
        store
            .postings("only_a")
            .unwrap()
            .iter()
            .all(|p| p.doc == "a1")
    );
    assert!(store.postings("unseen").unwrap().is_empty());

    assert_eq!(store.df("shared").unwrap(), 3);
    assert_eq!(store.df("only_a").unwrap(), 1);
    assert_eq!(store.df("only_c").unwrap(), 1);
    assert_eq!(store.df("unseen").unwrap(), 0);
}

#[test]
fn vector_roundtrips_when_present() {
    let path = tmp_path("vector-roundtrip");
    let store = MemStore::open(&path).unwrap();

    let mut with_vec = put_rows("a1", 3);
    with_vec.vector = Some((vec![i8::MIN, -1, 0, 1, i8::MAX], 0.25));
    let without_vec = put_rows("b2", 4);

    store.put(&with_vec).unwrap();
    store.put(&without_vec).unwrap();

    let got = store
        .vector("a1")
        .unwrap()
        .unwrap_or_else(|| panic!("vector must round-trip"));
    assert_eq!(got.0, vec![i8::MIN, -1, 0, 1, i8::MAX]);
    assert!((got.1 - 0.25).abs() < f32::EPSILON);

    assert!(store.vector("b2").unwrap().is_none());
    assert!(store.vector("missing").unwrap().is_none());

    // dims recorded on first vector put only.
    assert_eq!(store.dims().unwrap(), 5);

    // Batch scan returns both vector lane entries in key order (b2 has no
    // vector, so only a1 appears).
    let batch = store.vector_batch("", 10).unwrap();
    assert_eq!(batch.len(), 1);
    assert_eq!(batch[0].0, "a1");
}

#[test]
fn edges_and_concepts_reflect_puts() {
    let path = tmp_path("edges-concepts");
    let store = MemStore::open(&path).unwrap();

    let mut a = put_rows("a1", 3);
    a.edges = vec![("rust".to_string(), 2), ("kv".to_string(), 1)];
    let mut b = put_rows("b2", 4);
    b.edges = vec![("rust".to_string(), 3), ("bm25".to_string(), 1)];

    store.put(&a).unwrap();
    store.put(&b).unwrap();

    // Per-doc edges keyed by (concept, doc_id): each doc's own rows.
    let rust = store.edges("rust").unwrap();
    assert_eq!(
        rust.len(),
        2,
        "one edge row per doc that carried the concept"
    );
    assert!(rust.iter().any(|(d, w)| d == "a1" && *w == 2));
    assert!(rust.iter().any(|(d, w)| d == "b2" && *w == 3));

    assert!(
        store
            .edges("kv")
            .unwrap()
            .iter()
            .any(|(d, w)| d == "a1" && *w == 1)
    );
    assert!(store.edges("unseen").unwrap().is_empty());

    let concepts = store.concepts().unwrap();
    assert_eq!(concepts, vec!["bm25", "kv", "rust"]);
}

#[test]
fn archive_then_archived_roundtrips() {
    let path = tmp_path("archive-roundtrip");
    let store = MemStore::open(&path).unwrap();
    let rows = put_rows("a1", 7);
    store.put(&rows).unwrap();

    // archive() on an absent id is a no-op.
    store.archive("nope").unwrap();

    store.archive("a1").unwrap();

    assert_eq!(
        store.archived("a1").unwrap().as_deref(),
        Some(rows.doc.as_slice())
    );
    assert!(store.archived("nope").unwrap().is_none());

    // archive() removes the doc from mem_doc (it does not touch n_docs/
    // postings/df meta), so doc() is None and doc_ids() excludes it.
    assert!(store.doc("a1").unwrap().is_none());
    assert!(store.doc_ids().unwrap().is_empty());
}

#[test]
fn doc_ids_and_n_docs_stay_consistent_across_reopen() {
    let path = tmp_path("reopen-consistency");
    {
        let store = MemStore::open(&path).unwrap();
        for id in ["a1", "b2", "c3"] {
            store.put(&put_rows(id, 4)).unwrap();
        }
    }
    // Fresh open of the same path: state survives.
    let store = MemStore::open(&path).unwrap();
    let ids = store.doc_ids().unwrap();
    assert_eq!(
        ids,
        vec!["a1".to_string(), "b2".to_string(), "c3".to_string()]
    );
    assert_eq!(store.n_docs().unwrap(), 3);
    assert_eq!(store.sum_dl().unwrap(), 12);
    assert_eq!(
        store.doc("b2").unwrap().as_deref(),
        Some(b"doc-bytes-b2".as_slice())
    );
}
