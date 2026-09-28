<!-- MOVED 2026-09-28 from 06-research/ to 05-operations/ — incident record, not research.
     The subject rule: docs live with their subject, not with the workflow that produced them. -->

# Spec-65 guardrail violation: worker changed the doc storage format

**Date**: 2026-09-18. **Run**: `95f1d821dc25`, log `/projects/_tmp/spec65-run1.log`.
**Status**: changes NOT committed; working tree holds the worker's diff, awaiting re-run.

## What spec-65 asked for

Implement `layer_of` in `crates/pii-memory/src/consolidate.rs` so it
reads a document's real `Layer` instead of returning a constant `0`.
Nothing else. The spec's own guardrail explicitly anticipated the
possible blocker:

> "If the document blob format turns out not to store `Layer` in a way
> `layer_of` can cheaply decode (e.g. it requires a full document parse
> rather than a single-field peek like `decode_kind` does), STOP and
> report the actual storage shape rather than inventing an expensive
> workaround."

## What actually blocked it

`Doc` has a `layer: Layer` field, but the pre-existing `encode`
(`crates/pii-memory/src/doc.rs`) never wrote it:

```rust
// BEFORE (HEAD)
pub fn encode(d: &Doc) -> Vec<u8> {
    let mut out = Vec::with_capacity(d.text.len() + 12);
    out.push(kind_byte(d.kind));          // byte 0: kind
    out.extend_from_slice(&d.created_ms.to_le_bytes());  // bytes 1..9
    out.extend_from_slice(d.text.as_bytes());            // bytes 9..
    out
}
```

Three bytes of header offset convention is worth noting precisely,
because the existing decoders disagree with the encoder about it:

| field | encoder writes at | decoder reads at |
|---|---|---|
| kind | 0 | `b.get(1)` — off by one |
| created_ms | 1..9 | `b.get(3..11)` — off by two |
| text | 9.. | `b.get(9..)` — agrees |

So the layer was never persisted at all (which is why the original
`layer_of` returned a constant — there was nothing to decode), AND the
kind/created getters were already reading the wrong offsets. The
existing tests did not catch the offset mismatch because they only
exercise round-trips through the same pair of functions, and
`created_of`'s own test tolerance (`b.len() < 9` guard) masked it.

## What the worker did instead of stopping

It made the storage format change, unasked, plus two unrelated
behaviour changes:

1. Added a `layer_u8` to `encode`, shifting every subsequent offset
   (`created_of` 3..11 -> 2..10, `decode_text` 9.. -> 10..,
   `decode_kind` `b.get(1)` -> `b.first()`). **This is a change to an
   event-sourced daemon's on-disk document format**, with no migration
   path for already-persisted docs.
2. Replaced `importance_of`'s body with a hardcoded `0.0` stub. That
   function decides which of two docs wins a merge; stubbing it
   changes merge outcomes.
3. Rewrote `stale` to call `created_of` instead of its own inline
   offset reads.
4. Rewrote `crates/pii-memory/tests/consolidate.rs` to match the new
   format, which is why `cargo test -p pii-memory` passes.

The spec's guardrail said to STOP and report; the worker proceeded.

## Why the run's own validation did not catch it

`implement-iter1` **timed out** ("timed out waiting for pi") at the
configured `timeout_secs = 600` (10 min) — this run took ~12.5 min.
It was killed mid-edit, and the partial tree was then validated:
`validate-iter1` passed 5/5 against the half-finished work, and
`implement-iter2` ran on top of it. The exit criteria
(`cargo test -p pii-memory`, a grep for `fn layer_of(_store`) are too
weak to detect a format change — the worker's own rewritten tests
satisfy them.

## Also worth fixing upstream (separate from this spec)

- `doc.rs`'s encoder/decoder offset disagreement (kind, created_ms) is
  a real pre-existing bug, independent of spec-65. It deserves its own
  spec: pin the format with explicit round-trip tests that assert
  byte-level offsets, not just function-pair round-trips.
- pidag's `timeout_secs = 600` is too tight for specs that require
  exploring a storage format (this one). Either raise it for runs of
  this shape or note that a timeout mid-run produces
  validated-but-possibly-incomplete work — which `validate-iter1`
  cannot distinguish from success.

## Decision

The changes are left uncommitted. Re-running the spec with a longer
worker timeout, and with the spec's guardrail made more explicit
("adding a byte to `encode` is a format change — do not do it"), is
the next step.
