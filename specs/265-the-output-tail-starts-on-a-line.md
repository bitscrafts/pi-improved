# Spec 265: the output keeper's tail starts on a whole line — the test that proves it can fail

## Overview
**Project**: <repo>
**Phase**: production — observability of the coder node (spec 263 follow-up, found 2026-10-24 by mutating its implementation: mutation m5 survived all 13 tests)

Spec 263 R3 says the kept tail is "cut forward to the first newline after its start", so a line-based reader (`run_facts`) never sees half a JSON line at the seam. `OutputKeep::finish`
(`crates/pii-server/src/node_kinds/nodes/coder_output_keep.rs`) does it with `if let Some(i) = tail.iter().position(|&b| b == b'\n') { tail.drain(..=i); }`.

Mutation m5 replaces `tail.drain(..=i);` with `let _ = i;` (no cut). All 13 tests of `coder_output_keeps_the_tail.rs` still pass: they put the cut where the stream's line boundaries happen to fall,
so the seam is never mid-line. The rule is real and unprotected. Consequence if it regressed: the first tail line would be a fragment, `serde_json` would skip it as "not JSON", and the loss would be silent
unless the cut happened to land inside the `result` line itself (which is exactly the line the whole spec exists to keep).

## Verified Premises
- **CONFIRMED** (run) mutation m5 leaves `coder_output_keeps_the_tail` at `13 passed; 0 failed`; restored, 13 pass. m1 (no tail), m2 (no head), m3 (count off by one), m4 (exact-bound boundary) and m6 (head not cut back) each turn at least one test red.
- **CONFIRMED** (read) `finish` computes `budget = OUTPUT_KEEP_BYTES - head.len() - sep.len() - marker(total).len()`, takes the last `budget` bytes of the ring, then drains up to and including the first `\n`; `dropped = total - head.len() - tail.len()` uses the length AFTER the drain, so a missing drain also changes `dropped_bytes`.
- **CONFIRMED** (read) the existing helpers in `coder_output_keeps_the_tail.rs` (`stream`, `lines_of`, `keep`, `marker_of`) build streams of whole lines of fixed width; none builds a stream whose seam falls mid-line on purpose.
- **INFERRED, MEASURE FIRST** the exact `OUTPUT_KEEP_BYTES` and `OUTPUT_HEAD_BYTES` the tests can import (the existing test file shows how it reaches them). Reuse that path; add no visibility.

## Requirements
- **R1.** New file `crates/pii-server/tests/coder_output_tail_starts_on_a_line.rs` (do NOT edit `coder_output_keeps_the_tail.rs`; copy the few helpers). No source file changes.
- **R2.** The tests construct streams where the tail's first byte is MID-LINE by computing the layout from the real constants: a stream of whole lines of a known width `W` with `W` chosen so that `total - tail_budget` is not a multiple of `W` (assert that in the test, so the setup cannot rot into a boundary case).

## TDD Contract
| Test name | Setup | Expected |
|---|---|---|
| `the_tail_never_starts_mid_line` | over-cap stream of numbered lines, layout asserted to put the raw tail start mid-line | the first line after the marker line is a complete line of the stream (parses as the JSON it was; equals one of the input lines) |
| `every_line_after_the_marker_is_whole` | same stream | every line after the marker parses as JSON (no fragment anywhere) |
| `a_seam_inside_the_result_line_still_yields_the_result` | the stream's last line is a LONG `result` line (about 3 KiB) sized so the raw tail start falls inside the line BEFORE it; plus a second case where it would fall inside the result line's neighbour | `run_facts` returns the result line's `is_error`, `turns`, `cost_usd`, `result_text` |
| `the_dropped_count_matches_the_cut_tail` | same stream | `len(head) + len(tail_after_marker) + dropped_bytes == total_in` (it moves when the drain is skipped) |
| `a_seam_exactly_on_a_line_start_is_not_cut_further` | layout where the raw tail start IS a line start | no whole line is lost: the first kept line is the one that starts there? (see note) |

Note for the last test: `finish` drains through the first `\n` regardless, so a raw start that is exactly a line start loses that one line too. State what the code does by measuring, assert exactly that (the first kept line is the line AFTER the one starting at the raw start), and record in the test's comment that this loses at most one line by design. If the measured behaviour contradicts this, STOP and report instead of asserting it.

**The requirement a plausible implementation satisfies only in appearance:** a test whose stream has the seam on a line boundary. It passes with or without the cut (that is how m5 survived). R2's assertion on the layout, and the first two tests, are what catch a missing cut.

## Exit Criteria
- [ ] `cargo test -q -p pii-server --test coder_output_tail_starts_on_a_line 2>&1 | grep -qE "^test result: ok\. [5-9] passed"`
- [ ] `cargo test -q -p pii-server --test coder_output_keeps_the_tail 2>&1 | grep -qE "^test result: ok\. 13 passed"`
- [ ] `git diff --quiet -- crates/pii-server/src crates/pii-server/tests/coder_output_keeps_the_tail.rs`
- [ ] `cargo clippy -q -p pii-server --all-targets -- -D warnings`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

## Guardrails
- Do NOT edit any file under `crates/pii-server/src` or any existing test: this spec adds a test file only.
- Do NOT add a dependency, a raw print macro, or an absolute path.
- Do NOT commit.
