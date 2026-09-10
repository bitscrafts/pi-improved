//! `pii-memory` (F4.1): BM25 + i8 vectors + graph lane fused by RRF. Zero
//! model calls, zero socket code, redb reached only through
//! `pii_store::MemStore` (deps-check invariant 5).

pub mod bm25;
pub mod consolidate;
pub mod decay;
pub mod doc;
pub mod embed;
pub mod graph_lane;
pub mod quant;
pub mod rrf;
pub mod search;
pub mod tokenize;
pub mod vector;

pub use decay::{LAMBDA_ATOM, LAMBDA_PERSONA, LAMBDA_SCENE, LAMBDA_SKILL};
pub use doc::{Doc, DocKind, Layer};
pub use embed::{Embedder, NullEmbedder, TrigramEmbedder};
pub use search::{MemoryHit, MemorySearchResult, MemorySearcher};
pub use tokenize::tokenize;
