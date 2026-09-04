//! Embedders (F4.6): deterministic offline trigram default, an Ollama HTTP
//! embedder through `Arc<dyn HttpTransport>` (no socket code), and a Null
//! embedder for the degraded path.

use std::sync::Arc;

use pii_abi::PiiError;

/// f32 wire contract cast (clippy-permitted at the single call site).
#[allow(clippy::cast_possible_truncation)] // reason: f32 wire contract
fn f32_t(v: f64) -> f32 {
    v as f32
}
use pii_core::HttpTransport;

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

/// The real backend: `POST {host}/api/embeddings` via the transport (F4.6).
pub struct OllamaEmbedder {
    host: String,
    model: String,
    transport: Arc<dyn HttpTransport>,
}

impl OllamaEmbedder {
    /// Builds from host/model/transport.
    #[must_use]
    pub fn new(
        host: impl Into<String>,
        model: impl Into<String>,
        transport: Arc<dyn HttpTransport>,
    ) -> Self {
        Self {
            host: host.into(),
            model: model.into(),
            transport,
        }
    }
}

impl Embedder for OllamaEmbedder {
    fn dims(&self) -> usize {
        0 // unknown until the first response
    }

    fn name(&self) -> &'static str {
        "ollama"
    }

    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, PiiError> {
        let mut out = Vec::with_capacity(texts.len());
        for t in texts {
            let body = serde_json::json!({"model": self.model, "prompt": t});
            let req = pii_abi::HttpRequest {
                method: pii_abi::HttpMethod::Post,
                url: format!("{}/api/embeddings", self.host),
                headers: vec![(
                    String::from("Content-Type"),
                    String::from("application/json"),
                )],
                body: serde_json::to_vec(&body).unwrap_or_default(),
                timeout_ms: 30_000,
            };
            let ctx = pii_core::Ctx::root(pii_core::CapSet::empty(), pii_core::Budget::unlimited());
            let resp = self.transport.request(&ctx, &req)?;
            if !resp.is_success() {
                return Err(PiiError::Unavailable(format!(
                    "ollama embeddings: {}",
                    resp.status
                )));
            }
            let v: serde_json::Value = serde_json::from_slice(&resp.body)
                .map_err(|e| PiiError::Invalid(format!("ollama embeddings: {e}")))?;
            let emb = v
                .get("embedding")
                .and_then(|e| e.as_array())
                .ok_or_else(|| PiiError::Invalid(String::from("ollama: no embedding")))?;
            out.push(emb.iter().filter_map(|x| x.as_f64().map(f32_t)).collect());
        }
        Ok(out)
    }
}
