//! Embedders (F4.6): deterministic offline trigram default and a Null
//! embedder for the degraded path.

use pii_abi::PiiError;

/// An embedding backend (F4.6).
pub trait Embedder: Send + Sync {
    /// Vector width (0 = degraded).
    fn dims(&self) -> usize;
    /// Backend name.
    fn name(&self) -> &str;
    /// Embeds a batch of texts.
    ///
    /// # Errors
    /// Backend-specific.
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, PiiError>;
}

/// FNV-1a over bytes.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// The default deterministic offline embedder (F4.6).
///
/// Dims 256; lowercase, pad with `_`, character 3-grams hashed with FNV-1a
/// into buckets, weight `sqrt(tf)`, L2-normalised. Captures morphology and
/// typos exact-token BM25 misses.
#[derive(Debug, Clone, Copy, Default)]
pub struct TrigramEmbedder;

impl Embedder for TrigramEmbedder {
    fn dims(&self) -> usize {
        256
    }

    fn name(&self) -> &'static str {
        "trigram"
    }

    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, PiiError> {
        Ok(texts
            .iter()
            .map(|t| {
                let lower = format!("_{}_", t.to_lowercase());
                let mut buckets = vec![0u64; 256];
                let chars: Vec<char> = lower.chars().collect();
                for w in chars.windows(3) {
                    let gram: String = w.iter().collect();
                    let idx = usize::try_from(fnv1a(gram.as_bytes()) % 256).unwrap_or(0);
                    if let Some(slot) = buckets.get_mut(idx) {
                        *slot += 1;
                    }
                }
                let v: Vec<f32> = buckets
                    .iter()
                    .map(|c| {
                        #[allow(clippy::cast_precision_loss)] // reason: count
                        // magnitude never exceeds f32 mantissa relevance.
                        let n = *c as f32;
                        n.sqrt()
                    })
                    .collect();
                let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-12);
                v.iter().map(|x| x / norm).collect()
            })
            .collect())
    }
}

/// Dims 0 — proves the degraded path (F4.6).
#[derive(Debug, Clone, Copy, Default)]
pub struct NullEmbedder;

impl Embedder for NullEmbedder {
    fn dims(&self) -> usize {
        0
    }
    fn name(&self) -> &'static str {
        "null"
    }
    fn embed(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>, PiiError> {
        Err(PiiError::Unavailable(String::from("null embedder")))
    }
}
