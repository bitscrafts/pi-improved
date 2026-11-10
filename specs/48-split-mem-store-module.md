# pi-improved — spec-48: split `pii-store/src/mem.rs` into `mem/{mod,write,read}.rs`

- **Project**: `<repo>`
- **Priority**: LOW — Phase 5.2, remainder of F-12 (warn-band paydown, NOT
  a hard failure). `pii-store/src/mem.rs` is 416 effective lines, in the
  380-450 warn band. This is cosmetic debt reduction, not a blocker.

## Overview

`crates/pii-store/src/mem.rs` implements `MemStore` as a single `impl
MemStore` block mixing write-path methods (`open`, `put`, `meta`,
`bump_meta`, `doc_exists`, `bump_df`, `archive`) and read-path/query
methods (`n_docs`, `sum_dl`, `dims`, `postings`, `df`, `doc`, `doc_ids`,
`vector`, `vector_batch`, `edges`, `concepts`, `archived`). This is a
**pure move-and-re-export refactor** — same discipline as specs 43-45: no
behavior change, no test deletions/weakenings, every `pub` item keeps its
exact signature and stays reachable at `pii_store::mem::MemStore::<method>`
exactly as today.

Rust allows a type's `impl` block to be split across multiple files within
the same module (as long as they're all in the same crate) via a directory
module — read `crates/pii-store/src/mem.rs` in full before starting; ground
truth already confirmed by the orchestrator:
- `MemStore` struct, `PutRows` struct, `Posting` struct, and a private `fn
  err(name, e) -> PiiError` helper sit before the `impl` blocks (lines 1-48
  approx — verify exact boundary yourself).
- Methods in order: `open` (56), `n_docs` (100), `sum_dl` (108), `dims`
  (116), `meta` (120, private), `bump_meta` (131, private), `put` (161),
  `doc_exists` (222, private), `bump_df` (233, private), `postings` (281),
  `df` (313), `doc` (328), `doc_ids` (343), `vector` (366), `vector_batch`
  (386), `edges` (420), `concepts` (449), `archive` (477), `archived` (508).
- `crates/pii-store/src/lib.rs` re-exports whatever it currently re-exports
  from `mem` — grep it and confirm the exact re-export list before
  finishing, do not guess.

## Requirements

### Functional

- R1. Convert `crates/pii-store/src/mem.rs` into a directory module
  `crates/pii-store/src/mem/` with `mod.rs`, `write.rs`, `read.rs`.
  `crates/pii-store/src/lib.rs`'s `mod mem;` (or `pub mod mem;` — check
  which) is unchanged; Rust resolves it to `mem/mod.rs` automatically.
- R2. `mem/mod.rs` keeps: the module doc comment, `MemStore` struct,
  `PutRows` struct, `Posting` struct, the private `err` helper, and `impl
  MemStore { fn open, fn n_docs, fn sum_dl, fn dims, fn meta, fn
  bump_meta }` (the "core lifecycle + basic counters" subset — these are
  small and tightly coupled to `open`'s table-creation logic, keep them
  together in the primary file).
- R3. `mem/write.rs` holds a SECOND `impl MemStore { fn put, fn
  doc_exists, fn bump_df, fn archive }` block — the actual data-mutation
  path. `doc_exists`/`bump_df` stay private (`fn`, not `pub fn`) exactly
  as today; Rust permits a private method on `impl MemStore` defined in
  a different file within the same module tree as long as it's declared
  `mod write;` from `mem/mod.rs` (NOT `pub mod write;`, since these are
  implementation-detail methods, not a public sub-API — verify this
  compiles with the correct privacy before finalizing).
- R4. `mem/read.rs` holds a THIRD `impl MemStore { fn postings, fn df, fn
  doc, fn doc_ids, fn vector, fn vector_batch, fn edges, fn concepts, fn
  archived }` — the read-only query path.
- R5. No test module exists in the original file (confirmed — grep
  `#[cfg(test)]` in `mem.rs` before starting to double check) — if you
  find one this spec's ground truth missed, move it verbatim per the
  established precedent (an inner `#![cfg(test)]` in its own
  `mem/tests.rs`, or inline in `mod.rs` — whichever `xtask lint-files`
  accepts, that support already exists from spec-43).
- R6. No public API surface changes: `pii_store::mem::MemStore` (or
  however `lib.rs` re-exports it) resolves identically after the split.
  Confirm every external call site of `MemStore` still compiles unchanged
  — grep `MemStore` workspace-wide first to know what depends on it.

### Non-Functional

- N1. Each new file individually satisfies `xtask lint-files` (≤450
  effective lines, ideally under 380).
- N2. `#![forbid(unsafe_code)]` unaffected — no `unsafe` in the original
  file, none introduced.
- N3. No new `unwrap()/expect()/panic!()/todo!()/unimplemented!()`.

## Architecture

```
crates/pii-store/src/
├── mem.rs                   (DELETED — becomes mem/)
└── mem/
    ├── mod.rs        — MemStore, PutRows, Posting, err() helper,
    │                    impl MemStore { open, n_docs, sum_dl, dims,
    │                    meta, bump_meta }
    ├── write.rs       — impl MemStore { put, doc_exists, bump_df, archive }
    └── read.rs        — impl MemStore { postings, df, doc, doc_ids,
                          vector, vector_batch, edges, concepts, archived }
```

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| (any existing tests exercising `MemStore` elsewhere in the workspace, e.g. `pii-memory`'s integration tests) | — | still pass, byte-for-byte same assertions — this file itself has no inline tests per R5's ground truth |

## Exit Criteria

- [ ] `[ ! -f crates/pii-store/src/mem.rs ]`
- [ ] `[ -f crates/pii-store/src/mem/mod.rs ]`
- [ ] `[ -f crates/pii-store/src/mem/write.rs ]`
- [ ] `[ -f crates/pii-store/src/mem/read.rs ]`
- [ ] `cargo test --workspace --lib --bins --tests`
- [ ] `cargo test --workspace --doc`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo fmt --all --check`
- [ ] `cargo run -q -p xtask -- silent-noop-check`
- [ ] `cargo run -q -p xtask -- lint-files`
- [ ] `! grep -rE "unwrap\(|expect\(|panic!|todo!|unimplemented!" crates/pii-store/src/mem/mod.rs crates/pii-store/src/mem/write.rs crates/pii-store/src/mem/read.rs`

## Guardrails

- G1. Do NOT change any method's body, signature, or visibility.
- G2. Do NOT touch `specs/` (read-only for the workhorse).
- G3. Do NOT commit — leave the diff in the working tree for the
  orchestrator to review and commit.
- G4. Do NOT touch `crates/pii-store/src/lib.rs` unless its `mem`
  module declaration or re-export genuinely needs a path change (it
  should not — directory modules resolve transparently).
- G5. If splitting the `impl MemStore` block across files reveals a
  private-visibility conflict Rust's module system won't allow (e.g. a
  private helper method needed across `write.rs`/`read.rs` that can't be
  reached from a sibling file), STOP and report rather than silently
  widening visibility — `pub(super)` is preferred over `pub(crate)`
  where it suffices, matching every prior split spec's own guardrail.
- G6. No `unsafe` code (`#![forbid(unsafe_code)]`, workspace-wide).
