# Spec: a skills build must not change how stored docs decode

## Overview
Review 076 §0 (2026-10-19): the uncommitted 210 work changes how EVERY stored document is decoded — `decode_kind` read
`b.get(1)` at HEAD and the new build reads differently, so adding a `DocKind` silently reinterprets every doc already in
memory. Measured consequence: the live store holds 279 documents; a mis-decode turns knowledge into noise with no error.

**Project**: /projects/pi-improved

## Requirements
1. Adding a `DocKind` variant must not change the byte layout or the decode of existing values.
2. A regression test decodes a fixture captured from the CURRENT production store before and after the change and asserts
   identical kinds and text.
3. Unknown discriminants decode to a REFUSAL naming the byte, never to a default variant.
4. The wire/serde name of an existing variant cannot change; new variants are appended only.

## Exit Criteria
- [ ] `cargo test -p pii-memory --test doc_decode_is_stable`
- [ ] `cargo test -p pii-memory --test unknown_kind_refuses`
- [ ] a fixture captured from `/root/.local/state/pii/memory.redb` decodes to the same kinds before and after

## Guardrails
- Do NOT renumber or reorder existing variants.
- Do NOT default an unknown byte to `Atom` — that is how a skill becomes a fact.
