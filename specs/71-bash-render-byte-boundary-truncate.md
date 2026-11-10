# Spec: 71 — bash render truncates on byte boundaries only

**Project**: <repo>
**Phase**: intervention batch A — proven bug D2 (report 043 §6)
**Status**: NOT STARTED (worker lane)
**Derives from**: `_docs/06-research/043-opus-full-review-2026-10-06.md` (candidate 2, D2)

## Overview

`render` (crates/pii-tools/src/bash.rs:112-119) truncates a **lossy-decoded
String** at a byte index: `s.truncate(MAX_OUTPUT)` panics when the cut lands
mid-character (a multibyte char straddling the 64 KiB boundary). Proven today:
a 65535-ASCII + four-`ç` string panics with `byte index 65536 is not a char
boundary`. Line 106 (the combined-output path) truncates the BYTES first and
decodes after — correct; the `cap` closure at :112 does it in the wrong order.

## Requirements

### R1
`cap` truncates the byte slice at the boundary and THEN decodes, never
truncating a decoded String: `let b = if b.len() > MAX_OUTPUT { truncated =
true-ish marker path } … String::from_utf8_lossy(&b[..MAX_OUTPUT])`.

### R2
Behaviour on ASCII-only output is unchanged (identical bytes out; same marker
appends). Existing bash tests stay green.

### R3
The marker must still appear whenever output was actually cut. If a multibyte
char straddles the boundary, the cut may drop up to 3 bytes of it — that is
ACCEPTABLE (lossy decoding already drops partial tails); document it in one
line. Do NOT attempt to round the cut to a char boundary (keeps the cap
byte-exact at MAX_OUTPUT).

### R4
No signature changes; no new deps; `#![forbid(unsafe_code)]` holds; clippy
`-D warnings` + fmt clean.

## TDD Contract (ACCEPTANCE)

| Test | Input | Expected |
|---|---|---|
| `renders_multibyte_output_at_the_cap` (NEW, red today — add to bash.rs tests) | build `ProcOutput` with stdout = 65535 ASCII bytes + 4×`ç` (len 65539) | `render` returns Ok; `output` field length ≤ MAX_OUTPUT + marker; NO panic |
| boundary ASCII unchanged | 70000 ASCII bytes | cut at exactly 65536, marker appended |
| both streams capped independently | stdout+stderr each > cap | each field ≤ cap; combined path unaffected |

## Exit Criteria

> (Verified FAILING on the pre-implementation tree; after implementation all must pass.)

- [ ] `cargo test -p pii-tools --lib renders_multibyte 2>&1 | grep -q 'test result: ok. 1 passed'`
- [ ] `cargo test -p pii-tools --lib 2>&1 | grep -q 'test result: ok. 34 passed; 0 failed'`
- [ ] `cargo clippy -p pii-tools --all-targets -- -D warnings` (exit 0)
- [ ] `cargo fmt --all --check`

## Implementation Notes (worker)

- The one-line change shape: in `cap`, operate on the byte slice.
- The test module is at bash.rs `#[cfg(test)]` — add the test there; it must
  panic today (proved: the same shape panics in a standalone probe).