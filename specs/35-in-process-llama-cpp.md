# pi-improved — spec-35: in-process-llama-cpp

- **Project**: `<repo>`
- **Priority**: MEDIUM — add native in-process LLM support via `llama-cpp-2`
  (embed Granite 4.2 inside Pii), per research doc 015.

## Overview

Research doc `015-llm-in-process-llama-cpp.md` established that Granite 4.2's
non-thinking mode (`enable_thinking=False`) is **not controllable through
Ollama** — Ollama replaces the model's own Jinja chat template, so the probe
had to fall back to `llama-server --reasoning off`. The only way to get
Granite's non-thinking mode *inside the Pii binary* (no external server, no
`NET` capability) is to link llama.cpp directly via the `llama-cpp-2` crate.

This spec implements the doc's concrete integration sketch: a `LlamaCppProvider`
implementing the existing sync `ModelProvider` trait, a hand-built Granite
prompt formatter (so non-thinking is an explicit, testable function), and a
`llamacpp:<path>` spec arm in `provider_from_spec`.

## Requirements

### Functional

- **R1 (llama-cpp-2 dependency)**: add `llama-cpp-2` to `Cargo.toml`
  `[workspace.dependencies]` with `default-features = false, features = ["common"]`
  (no GPU/OpenMP features — CPU-only, matching the probe's 14-28 tok/s finding).
  `cmake` + `clang` are already installed on this box (verified 2026-09-10).
- **R2 (LlamaCppProvider)**: a new `crates/pii-model/src/llamacpp.rs` module
  with a `LlamaCppProvider` struct holding `Arc<LlamaModel>` + `Arc<LlamaBackend>`
  + `Mutex<LlamaContext<'static>>`, implementing `ModelProvider::chat` and
  `chat_streaming` via the tokenize→batch→decode→sample loop (doc 015 §1 API
  surface). `chat_streaming` feeds each `token_to_piece` through `on_delta`.
- **R3 (Granite prompt builder)**: a `granite::build_prompt(&[ChatMessage]) -> String`
  helper implementing the `<|start_of_role|>{role}<|end_of_role|>{content}<|end_of_text|>`
  wire format **by hand** (NOT `apply_chat_template`), so non-thinking mode is
  an explicit, unit-testable function (doc 015 §3 verification gap).
- **R4 (spec wiring)**: `provider_from_spec` gains a `"llamacpp"` arm, spec
  shape `llamacpp:<path-to-gguf>`. No transport needed (unlike `ollama:`).
  Env overrides: `PII_LLAMACPP_NUM_CTX` (default 4096), `PII_LLAMACPP_THREADS`
  (default: all cores).
- **R5 (capability fit)**: the provider load is a pure `fs_read` on the GGUF
  path — no `NET` capability required (doc 015 §2 capability fit).

### Non-Functional

- **N1**: the quality gate stays green; the test count only goes up.
- **N2**: `#![forbid(unsafe_code)]` stays in `pii-model` (the `unsafe` lives
  inside the `llama-cpp-2` dependency, not Pii's own source — doc 015 §1).
- **N3**: `silent-noop-check` (spec 24) stays green.
- **N4**: the automated test suite must NOT load a 2.2GB model (too heavy for
  CI). The real-model acceptance is a documented manual probe (R6).

## Architecture

`LlamaCppProvider` wraps the llama.cpp model/context. `chat` runs the
tokenize→batch→decode→sample loop synchronously (matching `ModelProvider`'s sync
contract — `GraphNode::run` is sync). `chat_streaming` is the same loop with
each `token_to_piece` forwarded through `on_delta`. The Granite prompt is built
by hand in `granite::build_prompt`, so non-thinking is explicit. `provider_from_spec`
dispatches `llamacpp:<path>` to construct the provider (no transport).

## TDD Contract

| Test | Given | Expects |
|---|---|---|
| `granite_prompt_builds_wire_format` (T1) | a 2-turn `[System, User]` transcript | the exact `<|start_of_role|>...<|end_of_text|>` string, no ` thinking` scaffolding |
| `granite_prompt_omits_thinking` (T2) | a transcript with an assistant turn | no ` thinking` / `enable_thinking` tokens in the output (non-thinking by construction) |
| `provider_spec_dispatches_llamacpp` (T3) | `provider_from_spec("llamacpp:/nonexistent.gguf")` | a clear `llamacpp:`-specific error (model load fails on missing file), NOT the generic "unknown model spec kind" |
| `llamacpp_requires_path` (T4) | `provider_from_spec("llamacpp:")` (empty path) | `PiiError::Invalid` "llamacpp: spec needs a model path" |

T1/T2 are the acceptance tests — a plausible implementation uses
`apply_chat_template` (which may emit thinking scaffolding) or skips the
hand-built prompt. T3/T4 prove the dispatch arm exists and validates its input.

## Exit Criteria

Machine-checked (each >=1 test, non-vacuity `grep -qE "ok\\. [1-9]"`):

- [ ] `cargo test -p pii-model granite_prompt_ 2>&1 | grep -qE "test result: ok\\. [1-9]"` — wire format (R3)
- [ ] `cargo test -p pii-model provider_spec_dispatches_llamacpp 2>&1 | grep -qE "test result: ok\\. [1-9]"` — dispatch (R4)
- [ ] `cargo test -p pii-model llamacpp_requires_path 2>&1 | grep -qE "test result: ok\\. [1-9]"` — validation (R4)
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
  real-model acceptance is a manual probe, not a CI test.
- **G6 — do NOT add `unsafe` to Pii's own source** (N2). The `unsafe` lives
  inside the `llama-cpp-2` dependency.

## Files to Modify

- `Cargo.toml` — add `llama-cpp-2` to `[workspace.dependencies]` (R1).
- `crates/pii-model/Cargo.toml` — add `llama-cpp-2` dep (R1).
- `crates/pii-model/src/llamacpp.rs` — new: `LlamaCppProvider` (R2) + `granite`
  prompt builder (R3).
- `crates/pii-model/src/lib.rs` — `provider_from_spec` `"llamacpp"` arm (R4);
  `pub mod llamacpp;` re-export.
- `crates/pii-model/src/` — the TDD contract tests (T1-T4).
