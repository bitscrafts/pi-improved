# Spec: a stream close is not an error (and a success terminal must not carry one)

## Overview
MEASURED 2026-10-18 on the DEPLOYED daemon (4182856bcc1b) against ollama-cloud glm-5.3-flash:

    {"kind":"assistant_delta","text":"GLM"}
    {"kind":"node_finished","node":"agent","port":"err",
     "value":{"error":"unavailable: net: read: peer closed connection without sending TLS close_notify"}}
    {"kind":"run_finished","terminal":{"kind":"success","output":{"error":"unavailable: net: read: ..."}}}

The answer arrived and was TRUNCATED: asked for PONG-GLM, pii received GLM plus a transport error. Two defects in one
line - a peer closing the stream without TLS close_notify is treated as an error instead of end-of-stream, AND the run
finished as success while carrying that error.

**Project**: /projects/pi-improved

## Requirements
1. A peer close without close_notify is END OF STREAM, not failure, when at least one complete frame was received: the
   answer is kept and the run is success with no error payload.
2. A close BEFORE any complete frame remains an error (net: read) - silence is not an answer.
3. terminal.kind = success must not carry an error payload. Either the run failed, or the error is a warning field. A
   success terminal with output.error is the shape that hides failures.
4. The behaviour is identical for every hosted provider (openai, anthropic, gemini): this is the net layer, not a quirk.

## Exit Criteria
- [ ] `cargo test -p pii-net --test stream_close_is_eof`
- [ ] `cargo test -p pii-graph --test success_carries_no_error`

Note on criterion 2 (2026-10-19, orchestrator): it was written against `pii-model`, which holds provider wire
code and composes no terminal — the crate that decides a run's terminal is `pii-graph`
(`run/run_helpers.rs::terminal_determination`), so the criterion could never have been satisfied there. The
requirement is unchanged; only the crate that must prove it is corrected. The test drives a real graph run in
which the last finished node took a non-`ok` port carrying `{"error": ...}` and asserts the terminal is not a
`success` that carries that error.

## Guardrails
- Do NOT weaken the gate, a budget or any existing control to make this pass.
- Do NOT make a truncated answer look complete: keep the frames received, say the stream ended, never invent tokens.
- Do NOT special-case ollama-cloud; the same rule must hold for any provider.
- The criterion's test must drive the production path (`NetService::request_streaming`, or `Conn::read` from
  inside the crate). A test that re-states the guard in its own body verifies nothing: measured 2026-10-19, an
  interrupted worker's `tests/stream_close_is_eof.rs` carried the mapping as `if false && (…)` and passed
  whichever way the real code was written.
- A green `cargo test` is not a green gate: that same file returned rc=0 while the tree's clippy step reported
  31 errors. `bash quality-gate.sh --routine` is the authority, and the item is not done until it is green.

## Closure

**Partial, 2026-10-19 (orchestrator).** An interrupted worker left the net-layer half in the tree: `Conn::read`
maps an abrupt TLS close to `Ok(0)`, and `conn_close_tests.rs` drives the real `Conn::read` over a real TLS
socket (both its cases pass). The item is NOT done, on two measured counts:

- `cargo test -p pii-net --test stream_close_is_eof` returned `2 passed; 0 failed`, rc=0 — but that test does
  not drive the production path (it mirrors the guard behind `if false`), and the same tree fails
  `quality-gate.sh` with 31 clippy errors in that file. A verify that passes on a gate-red, self-referential
  test is a false green, not progress.
- Requirement 3 (a success terminal must not carry an error) is unimplemented: `pii-graph`'s
  `terminal_determination` still returns `Success` with the last node's value when that node took the `err`
  port. The spec's second criterion named `pii-model`, which composes no terminal at all; it is corrected
  above to `pii-graph`, the crate that does, with the requirement unchanged.

The work is committed as a work-in-progress (not as done) and the item is back in the queue with a brief at
`_tmp/autonomy/briefs/191-stream-close.md`.
