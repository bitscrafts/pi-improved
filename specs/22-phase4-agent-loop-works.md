# pi-improved — spec-22: phase4-agent-loop-works

- **Project**: `/projects/pi-improved`
- **Priority**: P4.7.0 — opus's highest-leverage next step. Every P4.7 capability (localize, patch, validate, escalate, promote, research, PR) is a tool loop over `bash`/`grep`/`edit`. The agent loop currently does NOT function against a real model (three code-verified defects).
- **Status**: SPECIFIED — not yet dispatched · *(lifecycle: SPECIFIED → IN PROGRESS
  on dispatch → IMPLEMENTED — awaiting review → DONE after the orchestrator
  commits and updates HANDOFF.md)*
- **Source**: opus full-codebase review `_docs/06-research/010-pii-full-review-pass1.md` §F (2026-09-08)
- **Depends-On**: spec 20 (resume scope-root, approval-gate, heal-watch) — landed. This fixes what the review proved broken.

---

## Verified Premises

- `crates/pii-model/src/ollama.rs:228-230` — `OllamaProvider::chat_streaming` returns `ChatResponse { text, tool_calls: Vec::new(), ... }` — **A1: tool_calls hardcoded empty**. `AgentNode::run` (`agent.rs:229`) then sees `resp.tool_calls.is_empty()` always true → returns on `ok` at turn 1. `run_tool_calls` unreachable in production.
- `crates/pii-server/src/agent.rs:412-417` — `call_tool_bounded` substitutes `Ctx::root(CapSet::empty())` for the caller ctx → **A2: bash/grep/glob** (which declare `timeout()`) always get `capability denied`. The untimed tools (fs/edit) keep the real ctx → why Slice-1 heal passeed.
- `crates/pii-net/src/service.rs:395-418` — `request_streaming` does NOT dechunk `Transfer-Encoding: chunked` (proven vs real Ollama) → `on_chunk` first line is a hex size → **A3: `on_delta` never fires, `AssistantDelta` never emitted**. Also no cross-chunk line buffer.
- `crates/pii-model/src/ollama.rs:237-245` — `message_value` serialises ONLY `role`+`content` → **A7: tool_calls/tool_call_id dropped on the wire**, so multi-step tool use degrades (model re-requests the same tool).
- `crates/pii-server/src/agent.rs:`  — `call_tool_bounded` exists; `ctx` is `Send + Sync` + cloneable (`ctx.rs:36`), so cloning the caller ctx into the worker thread is the fix.
- `xtask/src/main.rs:63-81` — `run_ci` runs deps/net/lint/proto/bench/fmt/clippy/test; **E4: neither `reachability-check` nor `doc-drift-check` is in CI**.
- `crates/pii-mcp/src/main.rs:155` — server parks forever on `pending().await`; `run_client` inherits stderr and orphans the server. **D: the client hang is the server never exiting.** `RunningService::waiting()` exists (rmcp-3.2.0/src/service.rs:1093).

---

## Overview

Opus's full-codebase review verdict: **NEEDS WORK**. The substrate is sound, but
P4.1b/4.1c regressions mean **the agent's tool loop does not function against a
real model provider**. Three critical/high defects — tool calls never reach the
loop (A1), timed tools run with empty capabilities (A2), and streaming emits no
deltas because chunks aren't dechunked (A3) — plus tool-call structure dropped
on the wire (A7) and `run_ci` missing the self-improvement gates (E4).

**The non-negotiable exit criterion is a LIVE model run** (local Ollama):
the agent must actually call `grep` then `bash`, and `pii graph trace <run_id>`
must show `ToolCall` + `ToolResult` events and at least two `AssistantDelta`
events. Not a fake transport, not a scripted provider — a real model, since
A1/A2/A3 all pass vacuously with fakes.

---

## Requirements

### Functional

- **R1 (A1 — accumulate tool_calls in streaming)**: `OllamaProvider::chat_streaming`
  must accumulate `message.tool_calls` across NDJSON lines (same as `text`) and
  return them in the `ChatResponse.tool_calls`. `OllamaMessage.tool_calls` is
  already declared + `#[serde(default)]`.
- **R2 (A2 — pass the caller ctx to the bounded tool thread)**: `call_tool_bounded`
  must clone the caller's `Ctx` into the worker thread instead of substituting
  `Ctx::root(CapSet::empty())`. This preserves the heal run's `scope_root` fs_write
  confinement — do NOT substitute a root capset (would turn confinement into escalation).
- **R3 (A3 — dechunk in request_streaming + cross-chunk line buffer)**: 
  `NetService::request_streaming` must dechunk `Transfer-Encoding: chunked`
  (reuse `read_chunked`'s state machine incrementally) so streaming and buffered
  bodies are byte-identical. AND the Ollama delta closure must keep a residual
  `String` line buffer across chunks so a mid-JSON-line TCP read doesn't drop it.
- **R4 (A7 — send tool_calls/tool_call_id on the wire)**: `message_value` must
  serialize `tool_calls` (for assistant messages) and `tool_call_id` (for tool
  messages) so multi-step tool use works. This becomes load-bearing the moment
  R1 is fixed.
- **R5 (E4 — add self-improvement gates to run_ci)**: add `reachability-check` +
  `doc-drift-check` to `run_ci` and to `USAGE` so they're enforced by the CI gate
  set, not opt-in.
- **R6 (D — bounded MCP server lifetime)**: replace `pending().await` in the MCP
  server `main` with `running.waiting().await`; set `command.stderr(Stdio::null())`
  and `running.cancel().await` in the client so the self-test is a clean exit code.

### Non-Functional

- **N1**: The LIVE-MODEL exit criterion is non-negotiable — a fake/scripted
  provider that passes all three defects vacuously is a FAIL, not a pass.
- **N2**: quality gate stays green; test count only goes up.
- **N3**: `xtask` gates stay green (deps-check/net-check).
- **N4**: do NOT substitute a root capability set anywhere — preserve confinement.

---

## Architecture

```
AgentNode::run  (streaming path)
  → OllamaProvider::chat_streaming   [R1/A1: accumulate tool_calls]
       → transport.request_streaming [R3/A3: dechunk]
            → on_chunk → Ollama delta closure [R3/A3: cross-chunk line buffer]
                 → on_delta → emit AssistantDelta
  → resp.tool_calls  [R1: now non-empty]
       → run_tool_calls → call_tool_bounded [R2/A2: clone caller ctx]
            → tool.call(ctx_clone) [R7/A7: wire tool_calls/tool_call_id]
```

**Key decision — the live-model gate is the acceptance test.** A1/A2/A3 all pass
vacuously with fakes (the default `chat_streaming` delegates to `chat` which
returns tool calls; recording providers return none). Only a real Ollama run —
called with a prompt that must trigger `grep` then `bash`, traced via
`pii graph trace` — proves the loop actually functions. This is the exact
failure shape opus warned about: fake-green hides a loop that does nothing real.

---

## TDD Contract

| id | test | given | expects |
|----|------|-------|---------|
| R1 | `ollama_streaming_accumulates_tool_calls` (pii-model unit) | NDJSON lines, one carrying `message.tool_calls` | `ChatResponse.tool_calls` is non-empty and matches |
| R2 | `bounded_tool_uses_caller_ctx` (pii-server unit) | a fake tool that asserts the ctx has FS_READ | `tool.call` receives a ctx WITH FS_READ (not the empty root) |
| R3 | `request_streaming_dechunks` (pii-net unit) | a chunk-framed body split at a mid-line boundary | `on_chunk` emits dechunked NDJSON lines; `AssistantDelta` emitted |
| R4 | `message_value_includes_tool_calls` (pii-model unit) | an assistant msg with tool_calls | serialized JSON has `tool_calls`; a tool msg has `tool_call_id` |
| R5 | `run_ci_includes_self_improvement` (xtask static) | run_ci body | invokes reachability-check + doc-drift-check |
| R6 | `mcp_server_exits_on_eof` (pii-mcp integration) | start server, close stdin | server exits cleanly (exit 0) |
| N1 | `LIVE agent calls grep then bash` (integration, live Ollama) | prompt that must grep then bash, `pii graph trace` | trace shows `ToolCall` + `ToolResult` and ≥2 `AssistantDelta` |

**N1 is the acceptance test.** A plausible implementation fixes A1/A2/A3 in code
but the live gate FAILS because the fixes don't actually work against a real
model (e.g. dechunking off-by-one, tool_calls accumulated wrong, ctx not
actually cloned). The gate MUST be run live.

---

## Exit Criteria

- [ ] `cargo build --workspace --all-targets` — compiles (R1-R6)
- [ ] `cargo test -p pii-model ollama_streaming` — tool-call accumulation (R1)
- [ ] `cargo test -p pii-server bounded_tool` — caller ctx used (R2)
- [ ] `cargo test -p pii-net request_streaming` — dechunking (R3)
- [ ] `cargo test -p pii-model message_value` — tool_calls on wire (R4)
- [ ] `cargo test -p xtask run_ci` — self-improvement gates in CI (R5)
- [ ] `cargo test --workspace` — 0 failures (N2)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` — 0 errors (N2)
- [ ] `cargo fmt --all --check` — clean (N2)
- [ ] `cargo xtask lint-files` — 0 hard errors (N2)
- [ ] `cargo xtask deps-check` — pass (N3)
- [ ] `cargo xtask net-check` — pass (N3)
- [ ] **LIVE GATE (non-negotiable)**: against the local Ollama, `pii graph trace` of a run shows `ToolCall` + `ToolResult` events for `grep` and `bash`, AND at least two `AssistantDelta` events. (N1)

**Prose criteria**:

1. Test counts pasted raw, one line per binary, **unsummed**.
2. The live gate is the acceptance test — a fake-green suite that passes all
   unit tests but fails the live Ollama run is a FAIL.

---

## Guardrails

- **G1 — do NOT edit this spec.** If it is wrong, STOP and report it.
- **G2 — do NOT commit.** Leave work in the working tree.
- **G3 — do NOT weaken, skip or delete an existing test.**
- **G4 — do NOT regenerate a pinned fixture.**
- **G5 — no hardcoded absolute paths.** Test artefacts under `_tmp/`.
- **G6 — report raw output, never summed totals.**
- **G7 — the live gate is non-negotiable.** A fake/scripted-only verification is a FAIL.
- **G8 — do NOT substitute a root capability set.** Preserve the caller ctx + its `scope_root` confinement (R2/N4).
- **G9 — do NOT widen the MCP server's exposure beyond pii's TOOLS** (control plane stays on UDS JSON-RPC).

### Error handling expectations

- A partial NDJSON line at a chunk boundary must be buffered and completed on the next chunk, never silently dropped (A3).
- A `tool_calls` accumulation that misses a tool call must not silently produce a truncated tool loop (A1).
- The bounded tool thread must return `PiiError::Timeout` if the tool exceeds its deadline, never hang (R2).
- The MCP server must exit cleanly on stdin EOF, never orphan a child process (R6).

---

## Files to Modify

| File | Change |
|------|--------|
| `crates/pii-model/src/ollama.rs` | accumulate tool_calls in `chat_streaming` (R1); cross-chunk line buffer (R3); `message_value` sends tool_calls/tool_call_id (R4) |
| `crates/pii-server/src/agent.rs` | `call_tool_bounded` clones the caller ctx (R2) |
| `crates/pii-net/src/service.rs` | dechunk in `request_streaming` (R3) |
| `crates/pii-mcp/src/main.rs` | bounded server lifetime + client stderr/cancel (R6) |
| `xtask/src/main.rs` | add reachability-check + doc-drift-check to `run_ci` + USAGE (R5) |

**Not modified**: `crates/pii-server/src/lib.rs` (session/resume), `crates/pii-store/` (no schema change), `crates/pii-tools/` (the tools are fine).
