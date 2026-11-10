# pi-improved — spec-36: in-process-llama-cpp-tool-calls

- **Project**: `<repo>`
- **Priority**: MEDIUM — add structured tool-call support to the in-process
  `LlamaCppProvider` (spec 35), so ds4-bench's B1/B3/B4 (tool-calling and
  multi-step autonomy) become meaningful against granite running inside Pii.

## Overview

Spec 35 added the in-process `LlamaCppProvider` (embed Granite 4.2 via
`llama-cpp-2`), but it returns `tool_calls: Vec::new()` — text-only. ds4-bench's
B1 (tool-calling), B3 (2-step autonomy), and B4 (3-step autonomy) all require
the model to emit **structured `tool_calls`**. This spec adds that, using
llama.cpp's constrained-generation machinery: `json_schema_to_grammar` +
`LlamaSampler::grammar` (both already available in the `common` feature spec 35
enabled).

The model is already proven capable of tool calls (it passed all 4 ds4-bench
benchmarks via the router). The gap is purely the in-process provider not
exposing them.

## Requirements

### Functional

- **R1 (tool-call grammar)**: when a `ChatRequest` carries `tools`, build a
  JSON-schema grammar from the `ToolSpec`s via `llama_cpp_2::json_schema_to_grammar`
  and add a `LlamaSampler::grammar` to the decode loop. The grammar constrains
  the model to emit a valid tool-call JSON object.
- **R2 (tool-call parsing)**: parse the model's generated JSON into
  `Vec<ToolCall>` (id, name, arguments). The wire shape is the OpenAI
  function-call form: `{"name": "...", "arguments": {...}}` (or the
  `{"function": {"name", "arguments"}}` wrapper — match what granite emits).
- **R3 (text vs tool-call)**: when `tools` is empty, keep the current text-only
  path (no grammar). When `tools` is present, the model may emit either a
  tool-call JSON or plain text — detect which and populate `ChatResponse`
  accordingly (`text` and/or `tool_calls`).
- **R4 (streaming)**: `chat_streaming` forwards text deltas as before; the
  accumulated `ChatResponse` carries the parsed `tool_calls` (same as the
  buffered `chat`).

### Non-Functional

- **N1**: the quality gate stays green; the test count only goes up.
- **N2**: `#![forbid(unsafe_code)]` stays in `pii-model`.
- **N3**: `silent-noop-check` (spec 24) stays green.
- **N4**: the automated test suite must NOT load a 2.2GB model. The real-model
  tool-call acceptance is a documented manual probe (`#[ignore]`).

## Architecture

`LlamaCppProvider::run` gains a branch: if `req.tools` is non-empty, build the
grammar from the tool schemas, add a `grammar` sampler to the chain, and after
generation parse the output as a tool-call JSON. A `granite::tool_call_schema`
helper builds the JSON schema from `ToolSpec`s; a `parse_tool_calls` helper
parses the model output into `Vec<ToolCall>`.

## TDD Contract

| Test | Given | Expects |
|---|---|---|
| `tool_call_schema_builds_from_specs` (T1) | two `ToolSpec`s | a JSON schema with both tool names + argument schemas |
| `parse_tool_calls_extracts_name_and_args` (T2) | a model output `{"name":"fs_read","arguments":{...}}` | `ToolCall { name: "fs_read", arguments: {...} }` |
| `parse_tool_calls_handles_wrapper` (T3) | a model output `{"function":{"name":"x","arguments":{...}}}` | the same `ToolCall` (wrapper form) |
| `empty_tools_keeps_text_path` (T4) | a `ChatRequest` with `tools: []` | no grammar is built; the text-only path is unchanged |

T1/T2 are the acceptance tests — a plausible implementation skips the grammar
or mis-parses the tool-call shape.

## Exit Criteria

Machine-checked (each >=1 test, non-vacuity `grep -qE "ok\\. [1-9]"`):

- [ ] `cargo test -p pii-model tool_call_schema_ 2>&1 | grep -qE "test result: ok\\. [1-9]"` — schema (R1)
- [ ] `cargo test -p pii-model parse_tool_calls_ 2>&1 | grep -qE "test result: ok\\. [1-9]"` — parsing (R2)
- [ ] `cargo test -p pii-model empty_tools_ 2>&1 | grep -qE "test result: ok\\. [1-9]"` — text path (R3)
- [ ] `cargo test --workspace` — 0 failures (N1)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` — 0 errors (N1)
- [ ] `cargo fmt --all --check` — clean (N1)
- [ ] `cargo xtask silent-noop-check` — green (N3)

## Guardrails

- **G1 — do NOT edit this spec.**
- **G2 — do NOT commit.** Leave work in the working tree for orchestrator review.
- **G3 — do NOT weaken, skip or delete an existing test.**
- **G4 — do NOT touch `specs/`** (read-only for implementers).
- **G5 — do NOT load a real 2.2GB model in an automated test** (N4). The
  real-model tool-call acceptance is a manual probe, not a CI test.
- **G6 — do NOT add `unsafe` to Pii's own source** (N2).

## Files to Modify

- `crates/pii-model/src/llamacpp.rs` — tool-call grammar + parsing (R1-R4);
  the TDD contract tests (T1-T4).
