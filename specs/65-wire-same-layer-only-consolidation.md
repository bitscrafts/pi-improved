# Wire same_layer_only to a Real Layer Check in Memory Consolidation

- **Project**: `/projects/pi-improved`
- **Crate**: `pii-memory`
- **Priority**: MEDIUM — a safety/correctness config flag silently
  does nothing in production, contrary to what its name promises.
  Sourced from fable review finding 8,
  `_docs/06-research/032-fable-pii-review-2026-09-18.md` ("Follow-up"
  section) — independently re-verified before writing this spec.

## Overview

`crates/pii-memory/src/consolidate.rs` has:

```rust
fn layer_of(_store: &MemStore, _id: &str) -> u8 {
    0
}
```

used at the guard `if cfg.same_layer_only && layer_of(store, a) !=
layer_of(store, b) { ... }` (line ~122). Since `layer_of` always
returns `0` for every document, the inequality can never be true, so
the guard the `same_layer_only` flag is supposed to enforce never
fires — cross-layer consolidation merges happen regardless of the
flag's value.

Both production callers set the flag to `true`
(`crates/pii-server/src/node_kinds/nodes/memory.rs` and
`crates/pii-server/src/daemon/methods.rs` — verify current line
numbers, search for `same_layer_only`), meaning production code
currently believes cross-layer merges are prevented when they are not.
The only existing test sets the flag `false`
(`crates/pii-memory/tests/consolidate.rs`), which is exactly why this
never showed up as a test failure — the one test path that would
catch it doesn't exercise the `true` branch meaningfully (with
`layer_of` always `0`, `true` and `false` behave identically either
way, so no existing test distinguishes them).

Ground truth (verify yourself, re-read the file — it has grown since
this spec was drafted):
- `crates/pii-memory/src/consolidate.rs` — `layer_of`, the
  `same_layer_only` guard, and `ConsolidateConfig`'s (or whatever the
  config struct is named — search for `pub same_layer_only: bool`)
  full field list.
- `crates/pii-memory/src/doc.rs` — `Doc.layer: Layer` (the real field
  this should read), and the existing `decode_kind`/`created_of`
  pattern (functions that decode ONE field from a `Vec<u8>` blob) as
  the precedent to follow for a new `decode_layer` function, if the
  document is stored in the same encoded-blob format those functions
  already decode.
- `crates/pii-store` — how `store.doc(id)` returns document bytes
  (`Option<Vec<u8>>`), matching what `decode_kind`/`created_of`/
  `decode_text` all already consume.

## Requirements

### Functional

- R1. Implement `layer_of(store: &MemStore, id: &str) -> u8` for real:
  fetch the document's bytes via `store.doc(id)` (same call
  `text_of`/`concepts_of` already make in this file) and decode the
  actual `Layer` value from them, following the exact pattern
  `decode_kind`/`created_of` use in `pii-memory/src/doc.rs` for
  decoding other fields from the same blob format. If no
  `decode_layer`-equivalent function exists yet, add one in `doc.rs`
  matching those functions' signature/style
  (`pub fn decode_layer(bytes: Option<Vec<u8>>) -> Layer` or `-> u8`
  depending on what's more consistent with the existing functions —
  check whether `Layer` already has a byte-value conversion, e.g. via
  its own `as u8` or a dedicated method, before inventing a new
  encoding).
- R2. Once `layer_of` reads the real layer, the `same_layer_only`
  guard's existing comparison logic (`layer_of(store, a) !=
  layer_of(store, b)`) needs no other changes — it was already correct
  in shape, only the input was wrong.
- R3. Add a consolidate test with two documents on DIFFERENT layers
  and `same_layer_only: true`, asserting NO merge occurs between them
  (this is the test that fails today, since `layer_of` currently
  can't distinguish layers).

### Non-Functional

- N1. Do NOT change `ConsolidateConfig`'s field name or the guard's
  boolean logic — only `layer_of`'s implementation.
- N2. The existing test with `same_layer_only: false` must continue to
  pass unchanged — that flag value's behavior (merge regardless of
  layer) is correct today and stays correct.
- N3. If adding `decode_layer` to `doc.rs`, match the exact function
  signature style of its neighbors (`decode_kind`, `created_of`) —
  same error handling, same `Option<Vec<u8>>` input shape.

## Architecture

Described inline in Overview above (file paths and function names given there constitute the architecture context for this change).

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| `layer_of_reads_real_layer_value` | a document written with `Layer::L1`, another with a different layer variant | `layer_of` returns values that differ between the two, not both `0` |
| `consolidate_same_layer_only_true_blocks_cross_layer_merge` | two docs on different layers, `same_layer_only: true` | consolidation does NOT merge them — this test fails on today's code (guard never fires) and passes after the fix |
| `consolidate_same_layer_only_false_still_merges_cross_layer` | same two docs, `same_layer_only: false` (existing test, unchanged) | still merges as before — confirms N2 |

## Exit Criteria

- [ ] `! grep -q "fn layer_of(_store" crates/pii-memory/src/consolidate.rs`
- [ ] `cargo test -p pii-memory`
- [ ] `cargo build -p pii-memory`
- [ ] `cargo clippy -p pii-memory -- -D warnings`
- [ ] `cargo fmt --check`

## Guardrails

- Do NOT modify `pii-server/src/node_kinds/nodes/memory.rs` or
  `pii-server/src/daemon/methods.rs` (the two production callers) —
  they already set the flag correctly; only the flag's enforcement was
  broken.
- Do NOT modify any file under `specs/`.
- No emojis, no AI-tool mentions in any comment or commit message.
- If the document blob format turns out not to store `Layer`
  in a way `layer_of` can cheaply decode (e.g. it requires a full
  document parse rather than a single-field peek like `decode_kind`
  does), STOP and report the actual storage shape rather than
  inventing an expensive workaround — the alternative (removing the
  flag entirely if it can't be cheaply implemented) may be the right
  call, but that decision should come back to the user/orchestrator,
  not be made unilaterally by the workhorse.
