# Spec 263: a run's captured agent output keeps its head AND its tail

## Overview
**Project**: <repo>
**Phase**: production — observability of the coder node (found 2026-10-24 on queue row A126, run `r000000000108`)

The coder captures an agent's stdout through `capture_bounded` (`crates/pii-server/src/node_kinds/nodes/coder_runtime.rs:267`), which keeps the FIRST `OUTPUT_KEEP_BYTES` (256 KiB) and discards the rest.
claude's `--output-format stream-json` stream starts with a `system/init` line and ENDS with the one line that matters: `{"type":"result", "is_error", "api_error_status", "result", "num_turns", "total_cost_usd"}`.
A long run overflows the cap and the closing line is exactly what is thrown away.

Measured, A126 (spec 261), a 338 s haiku run that exited 1: `agent_output_bytes = 262144` (the cap), `claude_is_error = "unknown"`, `claude_turns = -1`, `claude_cost_usd = ""`, `claude_session_id` present
(it is in the head). The journal cannot say why the agent exited 1, what it cost, or how many turns it took. Three things the operator has asked for depend on that line and are blind without it:
spec 253 (the journal carries claude's own failure text), spec 254 (an auth failure parks the lane instead of burning an attempt), and the run-budget accounting Opus named as the gate on any unattended fix loop (the coder must charge the journalled cost).

This spec keeps the head (so `system/init`: model and session id) and the tail (so the `result` line) within the SAME bound, and marks the gap.

## Verified Premises
- **CONFIRMED** (read) two capture sites keep only a prefix: the process path `capture_bounded` (`coder_runtime.rs:267-282`, a streaming read in 8 KiB chunks, `room = OUTPUT_KEEP_BYTES - len`) and the container path `demux` (`coder_runtime_api.rs:416-436`, the same `room` rule per stream). `StdoutCapture::finish` returns `String::from_utf8_lossy` of the buffer; stderr uses the same type (`:195`).
- **CONFIRMED** (read) the one parser of the claude stream is `run_facts` (`agent_claude.rs:28-65`): it reads line by line, `continue`s past a line that is not JSON, ignores a `type` it does not know (`_ => {}`), and takes `turns`, `cost_usd`, `is_error`, `result_text`, `api_error_status` from a `result` line and `session_id`, `model` from `system/init`.
- **CONFIRMED** (run, journal of `r000000000108`) `agent_output_bytes":"262144"`, `claude_is_error":"unknown"`, `claude_turns":-1`, `claude_cost_usd":""`, `stop_reason":"end_turn"`, `rc":1`, `quota_exhausted":"no`.
- **CONFIRMED** (read) `crates/pii-server/tests/coder_container_bounded.rs::the_logs_are_bounded` feeds 5 MiB of `x` through the container path and asserts `!run.output.is_empty()` and `run.output.len() <= KEEP`. It must keep passing UNEDITED, so the new total (head + marker + tail) must never exceed `OUTPUT_KEEP_BYTES`.
- **CONFIRMED** (read) `report.rs:104` journals `agent_output_head` (200 chars) and `report.rs:140` calls `run_facts(self.agent_output)`.
- **INFERRED, MEASURE FIRST** how tests reach `drain_stdout` and `run_direct`/`Fake` (the existing `coder_container_bounded.rs` imports show the idiom). Use the SAME paths; do not widen visibility of anything unless the existing tests already did. If the process path cannot be driven from a test without a visibility change, STOP and report which item needs it.

## Requirements
### Functional
- **R1. Same bound.** The captured text of one stream never exceeds `OUTPUT_KEEP_BYTES` (256 KiB) in bytes, marker included, on both paths. `OUTPUT_KEEP_BYTES` keeps its value.
- **R2. Under the bound: untouched.** An output of at most `OUTPUT_KEEP_BYTES` bytes is returned byte for byte, with no marker (including exactly `OUTPUT_KEEP_BYTES`).
- **R3. Over the bound: head, gap, tail.** Otherwise the result is `HEAD` + `MARKER` + `TAIL` where `HEAD` is the first `OUTPUT_HEAD_BYTES` (64 KiB) cut back to the last newline in it, `TAIL` is the last bytes that fit in the remainder cut forward to the first newline after its start, and `MARKER` is one line `{"type":"pii_output_gap","dropped_bytes":N}` followed by `\n`, where `N` is exactly `total_in - (len(HEAD) + len(TAIL))`. The marker is a valid JSON object so every line-based reader skips it by its unknown `type`. If the head holds no newline, the head is kept whole; if the tail holds no newline, the tail is kept whole (a single enormous line is not erased).
- **R4. Streaming, bounded memory.** The capture holds at most `OUTPUT_KEEP_BYTES` plus one read chunk at any time, whatever the output size (5 GiB through a reader must not allocate 5 GiB), and its result is independent of how the reader chunks the bytes (1-byte, 7-byte and whole-buffer reads give the same text).
- **R5. One implementation.** Both paths use the same new type, in a new file `crates/pii-server/src/node_kinds/nodes/coder_output_keep.rs`; neither keeps its own copy of the rule.
- **R6. The consumers see the tail.** `run_facts` on a capture of an over-cap stream whose last line is a `result` line returns that line's `result_text`, `turns`, `cost_usd` and `is_error`, and still returns the `session_id` and `model` of the first line.
- **R7. A read error keeps what was read.** A reader that fails after N bytes (the current behaviour: stop at the error) yields those N bytes, untouched if N is within the bound.
- **R8. Nothing else changes.** No event, journal key or return type changes; `agent_output_head` still carries the first 200 characters.

### Non-Functional
- No new dependency, no raw print, no absolute path. Files stay under the 450 effective-line gate: the new type goes in `coder_output_keep.rs`, and `coder_runtime.rs` / `coder_runtime_api.rs` shrink or stay the same size.
- Do not edit any existing test.

## Architecture
```
agent stdout --8 KiB chunks--> OutputKeep { head: first <=64 KiB, tail: ring of the last bytes, total_in } --finish--> HEAD(cut at \n) + MARKER + TAIL(cut at \n)
   process path : capture_bounded            (coder_runtime.rs)      both call the SAME type
   container path: demux, per stream          (coder_runtime_api.rs)
under the bound: finish() returns the bytes unchanged (no marker)
```

## TDD Contract
New file `crates/pii-server/tests/coder_output_keeps_the_tail.rs`. Tests feed bytes through `drain_stdout` (process path) with an in-memory reader, and through the container fake of `coder_container_bounded.rs` (container path), and RUN `run_facts` on the result.

| Test name | Setup | Expected |
|---|---|---|
| `a_small_output_is_untouched` | 1,000 bytes | identical, no `pii_output_gap` |
| `an_output_exactly_at_the_bound_is_untouched` | `OUTPUT_KEEP_BYTES` bytes | identical, no marker (CONTROL for the boundary) |
| `one_byte_over_the_bound_is_cut_and_marked` | `OUTPUT_KEEP_BYTES + 1` bytes of short lines | a marker line with `dropped_bytes` >= 1; `len <= OUTPUT_KEEP_BYTES` |
| `the_total_never_exceeds_the_bound` | 5 MiB of lines | `len <= OUTPUT_KEEP_BYTES` |
| `the_head_survives_so_the_session_and_model_are_read` | 5 MiB; line 1 is `system/init` with a session id and model | `run_facts` returns both |
| `the_tail_survives_so_the_result_line_is_read` | 5 MiB; last line is a `result` line with `is_error: true`, `api_error_status: 401`, `result: "LAST"`, `num_turns: 9`, `total_cost_usd: 0.25` | `run_facts` returns all five values |
| `the_dropped_count_is_exact` | 5 MiB, known length | `len(output) - len(marker line) + dropped_bytes == total_in` |
| `the_marker_is_a_json_object_with_a_type` | any over-cap output | the marker line parses as JSON with `type == "pii_output_gap"` |
| `the_result_does_not_depend_on_the_chunking` | the same 3 MiB through three readers: 7 bytes per read, 1,009 bytes per read, and the whole buffer in one read | the three outputs are identical |
| `a_reader_error_keeps_what_was_read` | a reader that returns 5,000 bytes then an error | those 5,000 bytes, untouched |
| `a_single_enormous_line_is_not_erased` | one 1 MiB line with no newline | non-empty; `len <= OUTPUT_KEEP_BYTES`; starts with the line's first bytes and ends with its last bytes |
| `the_container_path_keeps_the_tail_too` | the container fake with a 5 MiB framed log whose last line is a `result` line | `run.output` contains it and `len <= KEEP` |
| `memory_stays_bounded_for_a_huge_stream` | a reader that yields 512 MiB of lines without storing them | completes; `len <= OUTPUT_KEEP_BYTES` (the reader is generated, not allocated) |

**The requirement a plausible implementation satisfies only in appearance:** raising the cap (say to 4 MiB) or keeping the tail alone. Raising it passes every small-input test and the existing bound test's shape, and fails `the_tail_survives_so_the_result_line_is_read` the moment a run outgrows it; tail-only loses the `system/init` line, which `the_head_survives_so_the_session_and_model_are_read` catches. Both of those tests must feed the node's real 5 MiB stream and call the real `run_facts`.

## Exit Criteria
- [ ] `cargo test -q -p pii-server --test coder_output_keeps_the_tail 2>&1 | grep -qE "^test result: ok\. (1[3-9]|[2-9][0-9]) passed"`
- [ ] `cargo test -q -p pii-server --test coder_container_bounded 2>&1 | grep -qE "^test result: ok"`
- [ ] `cargo test -q -p pii-server --test coder_journals_the_claude_result 2>&1 | grep -qE "^test result: ok"`
- [ ] `test -f crates/pii-server/src/node_kinds/nodes/coder_output_keep.rs`
- [ ] `grep -q "pii_output_gap" crates/pii-server/src/node_kinds/nodes/coder_output_keep.rs`
- [ ] `cargo test -q -p pii-server --lib 2>&1 | grep -qE "^test result: ok"`
- [ ] `cargo clippy -q -p pii-server --all-targets -- -D warnings`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`
- [ ] `git diff --quiet -- crates/pii-server/tests/coder_container_bounded.rs crates/pii-server/tests/coder_journals_the_claude_result.rs`

## Guardrails
- Do NOT raise `OUTPUT_KEEP_BYTES`, change a journal key, or edit any existing test; a failing existing test is a finding to report.
- Do NOT widen the visibility of an existing item unless the existing tests already reach it that way (see the INFERRED premise).
- Do NOT add a dependency, a raw print macro, or an absolute path.
- Do NOT commit.

## Follow-ups (not this spec)
Journal `dropped_bytes` as its own field on `coder.exited`; the coder charging the journalled cost to the run budget (Opus, Q6); retrying a worker whose exit code is non-zero but whose `result` line says `is_error: false`.
