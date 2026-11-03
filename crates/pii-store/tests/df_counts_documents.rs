//! Spec 358 R2: a re-put of the same document leaves df unchanged, so BM25
//! IDF counts documents, not writes.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use pii_store::{MemStore, mem::PutRows};

fn doc(id: &str) -> PutRows {
    PutRows {
        id: String::from(id),
        doc: vec![1, 2, 3],
        terms: vec![(String::from("alpha"), 1)],
        vector: None,
        edges: Vec::new(),
        dl: 1,
        layer: 0,
    }
}

#[test]
fn a_document_put_twice_is_counted_once() {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "df-once-").unwrap();
    let store = MemStore::open(&dir.path().join("df.redb")).unwrap();
    store.put(&doc("doc-a")).unwrap();
    store.put(&doc("doc-a")).unwrap();
    assert_eq!(store.df("alpha").unwrap(), 1);
}
