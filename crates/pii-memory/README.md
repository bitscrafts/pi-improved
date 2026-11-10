# pii-memory

`pii-memory`: BM25 + i8 vectors + graph lane fused by RRF. Zero model calls, zero socket code, redb reached only through `pii_store::MemStore` (deps-check invariant 5).

Public modules: `bm25`, `catalogue`, `consolidate`, `decay`, `doc`, `embed`, `entity`, `graph_lane`, `quant`, `rrf`, `search`, `skill`, `tokenize`, `vector`.

Depends on: `pii-abi`, `pii-core`, `pii-observe`, `pii-store`.
Used by: `pii-cli`, `pii-server`, `xtask`.

23 source files; 8 integration test files under `tests/`. Test with `cargo test -p pii-memory`.
