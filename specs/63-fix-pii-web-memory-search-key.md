# Fix pii-web Memory Search Parameter Key Mismatch

- **Project**: `<repo>`
- **Crate**: `pii-web`
- **Priority**: HIGH — a currently-broken user-facing feature (the web
  UI's memory search page always fails). Sourced from fable review
  finding 5, `_docs/06-research/032-fable-pii-review-2026-09-18.md`
  ("Follow-up" section) — independently re-verified before writing
  this spec.

## Overview

`crates/pii-web/src/main.rs`'s `api_memory` handler forwards the
search term to the daemon as `{"query": q.q}`. The daemon's handler,
`crates/pii-server/src/daemon/methods.rs`'s `memory_search`, reads
`req.params.get("q")` and returns `ErrorCode::InvalidParams` with the
message `"memory.search needs 'q'"` when that key is absent — which
it always is, since `pii-web` never sends it. The CLI's own client
(`pii-cli/src/cmd_ctl.rs`, verify current line — search for
`MemorySearch` or `memory_search`) sends the correct key `{"q": q, "k": k}`.
The web UI's memory page therefore always renders an error response,
100% of the time, for every search.

Ground truth (verify yourself, re-read both files — this spec was
drafted from a specific line snapshot):
- `crates/pii-web/src/main.rs` — `api_memory` function, the
  `serde_json::json!({...})` call it builds for `call_daemon`.
- `crates/pii-server/src/daemon/methods.rs` — `memory_search`
  function's `req.params.get(...)` call.
- `crates/pii-cli/src/cmd_ctl.rs` — the CLI's own memory-search
  client call, as the reference for the CORRECT key shape.

## Requirements

### Functional

- R1. Change `api_memory`'s daemon call to send `{"q": q.q}` (or
  `{"q": q.q, "k": <value>}` if `MemoryQuery` already has a `k` field
  — check the struct definition and preserve any existing fields,
  just fix the key name) instead of `{"query": q.q}`.
- R2. Add an integration test that stands up a fake/mock UDS responder
  (or reuses whatever test double `pii-web`'s existing tests already
  have, if one exists — check `main.rs`'s `#[cfg(test)]` module before
  writing a new one) and asserts the actual JSON `pii-web` sends over
  the wire contains the key `"q"`, not `"query"`. This test must FAIL
  against the pre-fix code and PASS after.

### Non-Functional

- N1. Do NOT change the daemon's `memory_search` handler
  (`pii-server/src/daemon/methods.rs`) — it is correct; `pii-web` is
  the side with the bug.
- N2. Do NOT change the CLI's memory-search call
  (`pii-cli/src/cmd_ctl.rs`) — it already sends the correct key,
  serving as the reference implementation for this fix.
- N3. This is a one-key rename; do not restructure `api_memory` or
  `MemoryQuery` beyond what's needed to fix the key.

## Architecture

Described inline in Overview above (file paths and function names given there constitute the architecture context for this change).

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| `api_memory_sends_correct_param_key` | a `MemoryQuery { q: "test term" }` request through `api_memory` | the JSON sent to `call_daemon` contains `{"q": "test term"}`, not `{"query": "test term"}` |
| `api_memory_route_reaches_daemon_successfully` | (if a fake UDS responder is added per R2) a full round-trip through the route with a mock daemon that only accepts `"q"` | the mock daemon receives a valid request and the route returns success, not `InvalidParams` |

## Exit Criteria

- [ ] `grep -q '"q".*q\.q\|q: q\.q' crates/pii-web/src/main.rs` (verify the exact fixed form matches; adjust this criterion if the actual code shape differs — the intent is "confirm the key is q, not query")
- [ ] `! grep -q '"query".*q\.q' crates/pii-web/src/main.rs`
- [ ] `cargo test -p pii-web`
- [ ] `cargo build -p pii-web`
- [ ] `cargo clippy -p pii-web -- -D warnings`
- [ ] `cargo fmt --check`

## Guardrails

- Do NOT modify `pii-server/src/daemon/methods.rs` or `pii-cli/src/cmd_ctl.rs`.
- Do NOT modify any file under `specs/`.
- No emojis, no AI-tool mentions in any comment or commit message.
- If `pii-web` has no existing test infrastructure for mocking the UDS
  daemon connection (per R2), a minimal fake responder is acceptable —
  but keep it scoped to this one test; do not build a general-purpose
  test harness beyond what R2 needs.
