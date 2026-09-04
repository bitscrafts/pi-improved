//! BM25 (F4.5): standard Okapi over redb postings — no in-RAM inverted
//! index. Pure decision function: table-tested against hand-computed
//! scores.

use pii_abi::PiiError;
use pii_store::mem::MemStore;

use crate::tokenize::tokenize;

/// Okapi k1 (F4.5).
pub const K1: f32 = 1.2;
/// Okapi b (F4.5).
pub const B: f32 = 0.75;

/// BM25 idf (F4.5): `ln(1 + (N - df + 0.5) / (df + 0.5))`. Pure.
#[must_use]
pub fn idf(n_docs: u64, df: u64) -> f32 {
    let n = f32::from(u16::try_from(n_docs).unwrap_or(u16::MAX));
    let df = f32::from(u16::try_from(df).unwrap_or(u16::MAX));
    ((n - df + 0.5) / (df + 0.5)).ln_1p()
}

/// BM25 term score for one doc. Pure.
#[must_use]
pub fn tf_score(tf: u32, dl: u64, avg_dl: f32) -> f32 {
    let tf = f32::from(u16::try_from(tf).unwrap_or(u16::MAX));
    let dl = f32::from(u16::try_from(dl.min(u64::from(u16::MAX))).unwrap_or(u16::MAX));
    let avg = if avg_dl <= 0.0 { 1.0 } else { avg_dl };
    (tf * (K1 + 1.0)) / K1.mul_add(B.mul_add(dl / avg, 1.0 - B), tf)
}

/// One search hit.
#[derive(Debug, Clone, PartialEq)]
pub struct Bm25Hit {
    /// Doc id.
    pub doc: String,
    /// BM25 score.
    pub score: f32,
}

/// Runs a BM25 search over the store (F4.5).
///
/// # Errors
/// [`PiiError::Internal`] on a store failure.
pub fn bm25_search(
    store: &MemStore,
    query: &str,
    k: usize,
) -> Result<Vec<(String, f32)>, PiiError> {
    let terms = tokenize(query);
    let n = store.n_docs()?;
    let sum_dl = store.sum_dl()?;
    let avg = if n > 0 {
        #[allow(clippy::cast_precision_loss)] // reason: dl counts fit f32 exact
        let avg = sum_dl as f32 / n as f32;
        avg
    } else {
        1.0
    };
    let mut acc: Vec<(String, f32)> = Vec::new();
    for term in terms {
        let df = store.df(&term)?;
        if df == 0 {
            continue;
        }
        let w = idf(n, df);
        for p in store.postings(&term)? {
            let dl = doc_len(store, &p.doc);
            let s = w * tf_score(p.tf, dl, avg);
            if let Some(e) = acc.iter_mut().find(|(d, _)| d == &p.doc) {
                e.1 += s;
            } else {
                acc.push((p.doc, s));
            }
        }
    }
    acc.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    Ok(acc.into_iter().take(k).collect())
}

/// Document length for one doc: recompute from the stored text (the doc
/// payload IS the text; dl was recorded in meta only as the corpus sum).
fn doc_len(store: &MemStore, id: &str) -> u64 {
    let Ok(Some(bytes)) = store.doc(id) else {
        return 0;
    };
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return 0;
    };
    u64::try_from(tokenize(text).len()).unwrap_or(0)
}
