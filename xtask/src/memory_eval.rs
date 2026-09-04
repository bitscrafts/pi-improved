//! `xtask memory-eval` (F4.16): builds a fresh `MemStore` under
//! `_tmp/phase3/memeval/`, indexes the corpus, prints exactly four lane
//! lines; exit 0 iff hybrid > bm25 strictly and hybrid >= 0.75.

// Module is consumed only from `main.rs` binary paths and its own tests;
// `pub(crate)` inside a private module trips redundant_pub_crate under this
// lint config, so the lint is relaxed for the whole file.
#![allow(clippy::redundant_pub_crate)] // reason: module private by design

use pii_memory::bm25::bm25_search;
use pii_memory::doc::{Doc, DocKind, Layer, encode};
use pii_memory::embed::Embedder as _;
use pii_memory::embed::TrigramEmbedder;
use pii_memory::graph_lane::graph_search;
use pii_memory::quant::quantise;
use pii_memory::rrf::rrf;
use pii_memory::vector::vector_search;
use pii_store::MemStore;
use serde::Deserialize;

#[derive(Deserialize)]
pub(crate) struct CorpusLine {
    #[serde(rename = "id")]
    pub(crate) id: String,
    pub(crate) text: String,
    #[serde(default)]
    pub(crate) concepts: Vec<String>,
}

#[derive(Deserialize)]
pub(crate) struct QueryLine {
    pub(crate) q: String,
    pub(crate) relevant: Vec<String>,
}

/// Runs the eval; prints the four lane lines (F4.16).
///
/// # Errors
/// Fixture or store failure.
pub(crate) fn run(corpus_path: &str, queries_path: &str, top_k: usize) -> Result<(), String> {
    let corpus = std::fs::read_to_string(corpus_path).map_err(|e| format!("corpus: {e}"))?;
    let queries = std::fs::read_to_string(queries_path).map_err(|e| format!("queries: {e}"))?;
    let docs: Vec<CorpusLine> = corpus
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).map_err(|e| format!("corpus line: {e}")))
        .collect::<Result<_, _>>()?;
    let qs: Vec<QueryLine> = queries
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).map_err(|e| format!("queries line: {e}")))
        .collect::<Result<_, _>>()?;

    let dir = std::path::Path::new("/projects/pi-improved/_tmp/phase3/memeval");
    let _ = std::fs::create_dir_all(dir);
    let path = dir.join("memory-eval.redb");
    let _ = std::fs::remove_file(&path); // deterministic scratch name, cleaned first
    let store = MemStore::open(&path).map_err(|e| format!("store: {e}"))?;
    let emb = TrigramEmbedder;

    for d in &docs {
        let vec = emb
            .embed(std::slice::from_ref(&d.text))
            .ok()
            .and_then(|v| v.into_iter().next())
            .as_deref()
            .map(quantise);
        put_doc(&store, d, vec)?;
    }

    let mut recalls: [Vec<f64>; 4] = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
    for query in &qs {
        let r = eval_query(&store, emb, query, top_k)?;
        recalls[0].push(r.0);
        recalls[1].push(r.1);
        recalls[2].push(r.2);
        recalls[3].push(r.3);
    }
    let mean = |v: &[f64]| {
        #[allow(clippy::cast_precision_loss)] // reason: sample count, tiny values
        let n = v.len() as f64;
        v.iter().sum::<f64>() / n.max(1.0)
    };
    let (bm, ve, gr, hy) = (
        mean(&recalls[0]),
        mean(&recalls[1]),
        mean(&recalls[2]),
        mean(&recalls[3]),
    );
    println!("lane=bm25 recall@10={bm:.3}");
    println!("lane=vector recall@10={ve:.3}");
    println!("lane=graph recall@10={gr:.3}");
    println!("lane=hybrid recall@10={hy:.3}");
    if hy > bm && hy >= 0.75 {
        Ok(())
    } else {
        Err(format!(
            "MEMORY-EVAL FAIL: hybrid={hy:.3} bm25={bm:.3} (must be hybrid > bm25 and hybrid >= 0.75)"
        ))
    }
}

/// Evaluates one query across the four lanes; returns (bm25, vector, graph,
/// hybrid) recalls.
fn eval_query(
    store: &MemStore,
    emb: TrigramEmbedder,
    query: &QueryLine,
    top_k: usize,
) -> Result<(f64, f64, f64, f64), String> {
    let top10 = |ids: &[(String, f32)]| -> Vec<String> {
        ids.iter().take(top_k).map(|(d, _)| d.clone()).collect()
    };
    let bm10 = top10(&bm25_search(store, &query.q, top_k).map_err(|e| format!("bm25: {e}"))?);
    let qv = emb
        .embed(std::slice::from_ref(&query.q))
        .ok()
        .and_then(|v| v.into_iter().next());
    let vec10 = match qv.map(|v| vector_search(store, &v, top_k)) {
        Some(r) => top10(&r.map_err(|e| format!("vector: {e}"))?),
        None => Vec::new(),
    };
    let gr10 = top10(&graph_search(store, &query.q, top_k).map_err(|e| format!("graph: {e}"))?);
    let hyb10: Vec<String> = rrf(&[bm10.clone(), vec10.clone(), gr10.clone()])
        .into_iter()
        .take(top_k)
        .map(|(d, _)| d)
        .collect();
    let rel = &query.relevant;
    let recall = |top: &[String]| -> f64 {
        let hits = top.iter().filter(|t| rel.contains(t)).count();
        f64::from(u16::try_from(hits).unwrap_or(0))
            / f64::from(u16::try_from(rel.len().max(1)).unwrap_or(1))
    };
    Ok((recall(&bm10), recall(&vec10), recall(&gr10), recall(&hyb10)))
}

/// Puts one corpus doc through the store (single write txn per doc).
pub(crate) fn put_doc(
    store: &MemStore,
    d: &CorpusLine,
    vec: Option<(Vec<i8>, f32)>,
) -> Result<(), String> {
    let doc = Doc {
        id: d.id.clone(),
        layer: Layer::L1,
        kind: DocKind::Atom,
        text: d.text.clone(),
        concepts: d.concepts.clone(),
        importance: 1.0,
        created_ms: 0,
        last_used_ms: 0,
        evidence: None,
        archived: false,
    };
    let toks = pii_memory::tokenize::tokenize(&d.text);
    let mut terms: std::collections::BTreeMap<String, u32> = std::collections::BTreeMap::new();
    for t in &toks {
        *terms.entry(t.clone()).or_insert(0) += 1;
    }
    let edges = d.concepts.iter().map(|c| (c.clone(), 1u64)).collect();
    let rows = pii_store::mem::PutRows {
        id: doc.id.clone(),
        doc: encode(&doc),
        terms: terms.into_iter().collect(),
        vector: vec,
        edges,
        dl: u32::try_from(toks.len()).unwrap_or(u32::MAX),
    };
    store.put(&rows).map_err(|e| format!("put: {e}"))
}

/// Parses the two fixture files (used only by the anti-rigging test, F4.17).
///
/// # Errors
/// Malformed JSONL.
#[cfg(test)]
pub(crate) fn parse_fixtures(
    corpus_path: &str,
    queries_path: &str,
) -> Result<(Vec<CorpusLine>, Vec<QueryLine>), String> {
    let corpus = std::fs::read_to_string(corpus_path).map_err(|e| format!("corpus: {e}"))?;
    let queries = std::fs::read_to_string(queries_path).map_err(|e| format!("queries: {e}"))?;
    let docs: Vec<CorpusLine> = corpus
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).map_err(|e| format!("corpus line: {e}")))
        .collect::<Result<_, _>>()?;
    let queries: Vec<QueryLine> = queries
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).map_err(|e| format!("queries line: {e}")))
        .collect::<Result<_, _>>()?;
    Ok((docs, queries))
}
