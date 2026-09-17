# pi-improved — spec-53: direct `MemStore` contract tests (F-18/F-19)

- **Project**: `/projects/pi-improved`
- **Priority**: MEDIUM — Phase 5.3, closing F-19's specific claim: "no
  tests at all (inline or integration) verified for `pii-store/src/
  mem.rs`". This is a real gap: `MemStore` is exercised only INDIRECTLY
  through `pii-memory`'s higher-level tests (`consolidate.rs`,
  `lanes.rs`, `persistence.rs`), never directly at the `pii-store` crate
  level. `crates/pii-store/tests/store_contract.rs` exists but tests a
  DIFFERENT type (`pii_store::Store`, the run journal) — it does not
  touch `pii_store::mem::MemStore` at all. Verify this yourself before
  starting: `grep -c MemStore crates/pii-store/tests/store_contract.rs`
  should be 0.

## Overview

Write a new integration test file exercising `MemStore`'s full public
API directly — every method in `mem/mod.rs` (`open`, `n_docs`, `sum_dl`,
`dims`), `mem/write.rs` (`put`, `archive`), and `mem/read.rs`
(`postings`, `df`, `doc`, `doc_ids`, `vector`, `vector_batch`, `edges`,
`concepts`, `archived`) — WITHOUT going through `pii-memory`'s BM25/
consolidation/quantization layers, so a future `MemStore`-level bug is
caught at the crate boundary instead of only surfacing several layers
up in `pii-memory`'s own tests.

Ground truth already confirmed by the orchestrator (verify yourself
before starting):
- `MemStore::open(path: &Path) -> Result<Self, PiiError>`.
- `PutRows { id: String, doc: Vec<u8>, terms: Vec<(String, u32)>,
  vector: Option<(Vec<i8>, f32)>, edges: Vec<(String, u64)>, dl: u32 }`
  is the write-request struct — construct it directly in the test, no
  need to go through `pii-memory`'s doc-encoding helpers.
- `Posting { doc: String, tf: u32 }` (or similar — read the actual
  struct fields from `crates/pii-store/src/mem/mod.rs` before writing
  assertions).
- `put` is idempotent on `id` (re-put replaces the doc and its rows —
  confirmed by the doc comment on `write.rs`'s `put` method).
- `archive`/`archived` move a doc to/read from a separate archive table
  (`MEM_ARCHIVE`) — a doc that's `archive`d should still be readable via
  `archived(id)` but the exact interaction with `doc(id)` (does the
  regular read still work, or does archiving remove it from `MEM_DOC`?)
  needs to be read from the actual `archive`/`archived` method bodies
  before writing an assertion — do not guess this behavior.

## Requirements

### Functional

- R1. New file `crates/pii-store/tests/mem_store_contract.rs`. Each test
  owns its own database file under a `_tmp`-relative path resolved via
  `concat!(env!("CARGO_MANIFEST_DIR"), "/../../_tmp")` (per F-15's
  established convention — do NOT hardcode `/projects/pi-improved/_tmp`
  the way `store_contract.rs` does; that's a stale pattern predating the
  F-15 fix, not one to copy).
- R2. Test `put_then_read_doc_roundtrips`: construct a `PutRows` with a
  doc id, encoded bytes, a couple of `(term, tf)` postings, no vector, no
  edges, some `dl`; `put` it; `doc(id)` returns the same bytes back;
  `doc_ids()` includes the id; `n_docs()`/`sum_dl()` reflect the one doc.
- R3. Test `put_is_idempotent_on_id`: `put` the same id twice with
  DIFFERENT doc bytes the second time; `doc(id)` returns the SECOND
  bytes (replace semantics, not append/error); `n_docs()` is still 1
  (not 2) after the second put.
- R4. Test `postings_and_df_reflect_puts`: put 2-3 docs sharing a common
  term with different `tf` values; `postings(term)` returns all of them
  in the expected shape; `df(term)` returns the correct document
  frequency (number of distinct docs containing the term).
- R5. Test `vector_roundtrips_when_present`: put a doc WITH a
  `(Vec<i8>, f32)` vector; `vector(id)` returns it back
  byte-for-byte/value-for-value; put a doc WITHOUT a vector;
  `vector(other_id)` returns `None` (or whatever the actual `Option`
  shape is per the method signature).
- R6. Test `edges_and_concepts_reflect_puts`: put a doc with some
  `(concept, weight)` edges; `edges(concept)` returns the doc(s)
  connected to that concept; `concepts()` lists the distinct concept
  names seen.
- R7. Test `archive_then_archived_roundtrips`: put a doc, `archive(id)`
  it, then verify via `archived(id)` that the archived bytes are
  retrievable — read the actual `archive`/`archived` implementations
  first to know exactly what postcondition to assert (whether the doc
  disappears from `doc(id)`/`doc_ids()` after archiving, or stays
  visible in both places — this is a real design question the code
  answers, not something to assume).
- R8. Test `doc_ids_and_n_docs_stay_consistent_across_reopen`: put 2-3
  docs, close the `MemStore` (drop it), `MemStore::open` the SAME path
  again, confirm `doc_ids()`/`n_docs()` still reflect what was written
  — proves persistence survives a fresh handle, not just an in-memory
  cache.

### Non-Functional

- N1. No `unwrap()/expect()/panic!()` restriction does NOT apply inside
  this test file's own assertions/setup (standard for test code, matches
  every other test file's `#![allow(clippy::unwrap_used, ...)]` at the
  top) — but the PRODUCTION code under test (`crates/pii-store/src/
  mem/*.rs`) must not be touched by this spec at all; this is a
  test-only addition.
- N2. Tests must be independent (no shared mutable global state, no
  test-order dependency) — each test gets its own fresh database path.

## Architecture

No production code changes — this is a test-only addition:

```
crates/pii-store/tests/
├── store_contract.rs          (existing, unchanged — tests Store)
└── mem_store_contract.rs      (NEW — tests MemStore directly)
```

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| `put_then_read_doc_roundtrips` | One `PutRows` | `doc()`/`doc_ids()`/`n_docs()`/`sum_dl()` all reflect it |
| `put_is_idempotent_on_id` | Same id, two different payloads | Second payload wins; doc count stays 1 |
| `postings_and_df_reflect_puts` | 2-3 docs sharing a term | `postings()`/`df()` match the puts |
| `vector_roundtrips_when_present` | One doc with a vector, one without | Vector round-trips exactly; absent vector reads as empty/None |
| `edges_and_concepts_reflect_puts` | A doc with concept edges | `edges()`/`concepts()` reflect them |
| `archive_then_archived_roundtrips` | One doc, then archived | `archived()` returns it; behavior of `doc()`/`doc_ids()` post-archive matches what the actual `archive`/`archived` code does (read it first) |
| `doc_ids_and_n_docs_stay_consistent_across_reopen` | 2-3 docs, drop, reopen same path | State survives a fresh `MemStore::open` |

## Exit Criteria

- [ ] `[ -f crates/pii-store/tests/mem_store_contract.rs ]`
- [ ] `cargo test -p pii-store --test mem_store_contract`
- [ ] `cargo test --workspace --lib --bins --tests`
- [ ] `cargo test --workspace --doc`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo fmt --all --check`
- [ ] `cargo run -q -p xtask -- silent-noop-check`
- [ ] `cargo run -q -p xtask -- lint-files`
- [ ] `! grep -n "/projects/pi-improved/_tmp" crates/pii-store/tests/mem_store_contract.rs`

## Guardrails

- G1. Do NOT modify any file under `crates/pii-store/src/` — this is a
  test-only addition. If a test reveals what looks like a real bug in
  `MemStore`'s implementation, STOP and report it rather than silently
  "fixing" the production code to match a wrong assumption — the point
  of this spec is coverage, not a refactor.
- G2. Do NOT touch `crates/pii-store/tests/store_contract.rs` — it tests
  a different type (`Store`, not `MemStore`) and is out of scope.
- G3. Do NOT touch `specs/` (read-only for the workhorse).
- G4. Do NOT commit — leave the diff in the working tree for the
  orchestrator to review and commit.
- G5. No `unsafe` code (`#![forbid(unsafe_code)]`, workspace-wide).
