# Spec: 70 — The chunked decoder cannot be driven past its buffer

**Project**: /projects/pi-improved
**Phase**: intervention batch A — proven bug D1 (report 043 §6, verdict §1)
**Status**: NOT STARTED (worker lane)
**Derives from**: `_docs/06-research/043-opus-full-review-2026-10-06.md` (candidate 1, D1)

## Overview

`dechunk_step` (crates/pii-net/src/service/dechunk.rs:12-58) mis-handles a
chunked body delivered byte-at-a-time at five of the sixteen split points of
`b"5\r\nhello\r\n0\r\n\r\n"`. Proven RED on HEAD today (test
`every_split_point_decodes`, appended by the orchestrator — see TDD):

- split 7 (mid-size-line): returns `Invalid("http: truncated chunk")` — the
  `raw.get(data_start..data_start + size)` guard fires because `size`
  (0x35='5'? size parsed from the truncated hex "5" is fine; the real cause:
  rel points at the FIRST CRLF found, which at split 7 is the one INSIDE the
  data area, so data_start lands mid-chunk)
- split 8: `Invalid("http: bad chunk size")` (size line without CRLF yet is
  parsed as hex with a trailing partial)
- **split 9: `consumed 10 > pending 9` — the caller's `pending.drain(..consumed)`
  would PANIC** (this is the report's crash: a read ending after a chunk's CR
  advances pos past the buffer at :54-56 — `if raw.get(*pos) == Some(&b'\r')
  { *pos += 2; }` consumes a CRLF that is not fully present; a lone `\r` at
  buffer end advances pos by 2, past `raw.len()`)
- split 13/14 (mid-terminator): `Invalid("http: bad chunk size")` — the
  terminating chunk's `0\r\n\r\n` split mid-line is parsed as a size line.

Live path: every streamed model turn (pii-model/src/ollama.rs:201 streams
through this decoder; caller feeds read-sized slices and drains `consumed`).

## Requirements

### R1 — Split-safety (the fix, per report §12)
A step may only consume bytes whose framing is fully present:
- R1.1: a size line is parsed only when a CRLF exists **and the line is
  non-empty hex**; fewer bytes than a complete line ⇒ `Ok(())` (wait), never a
  parse error.
- R1.2: the completeness check is `data_start + size + 2 <= raw.len()`
  (data + trailing CRLF both present) before consuming; a lone trailing `\r`
  is NEVER consumed (`*pos = data_start + size` stands; the CRLF is consumed
  on the NEXT step when its `\n` has arrived).
- R1.3: the terminating chunk requires the full `0\r\n\r\n` (or terminator +
  trailer lines) before its position advances; a partial trailer line waits.
- R1.4: `find_crlf` may not find a CRLF that begins in the final byte pair
  without its `\n` (split 8 case: `\r` present, `\n` not) — the position
  search must ignore a window whose second byte is absent.

### R2 — No behaviour change on complete inputs
A complete, well-formed chunked body decodes identically to today (same bytes
out, same final pos) — the existing passing suite must stay green.

### R3 — Guardrails
- No new dependencies; no `unsafe`; clippy `-D warnings` clean; fmt clean.
- Do NOT change `total_len`'s size gate; do NOT touch http.rs framing.

## TDD Contract (ACCEPTANCE)

| Test | Input | Expected |
|---|---|---|
| `every_split_point_decodes` (ALREADY IN TREE, red) | `b"5\r\nhello\r\n0\r\n\r\n"` fed at every split k∈0..=16 with `consumed≤pending.len()` asserted | Green: decodes to `b"hello"` at every split; consumed never exceeds pending; drain never panics |
| oversized chunk still refused | `A\r\n` + 11 bytes | `Ok` at 12 bytes pending, `Err(body too large)` per `total_len` when complete |
| trailer chunk | `5\r\nhello\r\n0\r\nX-T: v\r\n\r\n` split at trailer mid-line | waits then completes, pos lands after final CRLF |

## Exit Criteria (checked on the pre-implementation tree, currently FAILING)

- [ ] `cargo test -p pii-net --lib every_split_point_decodes 2>&1 || echo FAIL-SHOWN && cargo test -p pii-net --lib every_split_point_decodes 2>&1 | grep -q 'test result: ok'`
- [ ] `cargo test -p pii-net --lib 2>&1 | grep -q 'test result: ok'`
- [ ] `cargo clippy -p pii-net --all-targets -- -D warnings 2>&1 | tail -1 | grep -q warning= && false || true`
- [ ] `cargo fmt --all --check`
- [ ] `grep -c 'data_start + size + 2 <= raw.len()' crates/pii-net/src/service/dechunk.rs` (the completeness condition exists)

## Implementation Notes (worker)

- Keep the function's signature and resumability contract.
- The five failing splits all come from two roots: (a) position advanced on a
  CRLF whose `\n` is absent (:54-56), (b) size-line parsed before its CRLF is
  fully in the buffer. Fix both; do not hand-wave "wait for more bytes" into
  error returns.