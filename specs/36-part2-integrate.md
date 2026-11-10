# Spec: spec-36 part2 — integrate tool-call grammar + parse into run()

**Project**: <repo>
**Phase**: 4 - spec 36 (in-process llama-cpp tool calls), micro-task part 2

## Overview

Part 1 (committed) added `crate::llamacpp_tools` with two pure helpers:
`build_schema_from_tools(&[ToolSpec]) -> String` and
`parse_tool_calls(&str) -> Vec<ToolCall>`. This part wires them into
`LlamaCppProvider::run` in `crates/pii-model/src/llamacpp.rs` so that when a
`ChatRequest` carries `tools`, the decode uses a grammar-constrained sampler
and the result populates `ChatResponse.tool_calls`.

You are editing ONE existing file: `crates/pii-model/src/llamacpp.rs`.
Make SMALL, PRECISE, SURGICAL edits only. Do NOT rewrite the file. Do NOT
restructure anything.

## Precedent (read-only reference, do not change)

`crates/pii-model/src/llamacpp_tools.rs` (created in part 1, committed) exposes:
- `pub fn build_schema_from_tools(tools: &[ToolSpec]) -> String`
- `pub fn parse_tool_calls(output: &str) -> Vec<ToolCall>`

Both are already `pub` and reachable as `crate::llamacpp_tools::{build_schema_from_tools, parse_tool_calls}`.

## Requirements

### R1 — import the helpers

Add to the existing `use` block at the top of `llamacpp.rs` (near the other
`use crate::{...}` at line 29):

```rust
use crate::llamacpp_tools::{build_schema_from_tools, parse_tool_calls};
```

Add it on its own line, right below the existing
`use crate::{ChatMessage, ChatRequest, ChatResponse, ModelProvider, Role};`

### R2 — grammar-constrained sampler when tools present

In the `run` method, REPLACE the sampler setup at line ~110:

```rust
        // Sample loop: greedy (deterministic) — matches the probe's
        // non-thinking, content>0 requirement without a reasoning trace.
        let mut sampler = LlamaSampler::chain_simple([LlamaSampler::greedy()]);
```

WITH this (preserving the comment then branching on tools):

```rust
        // Sample loop. When tools are requested, constrain generation to a
        // tool-call JSON via llama.cpp's grammar (R1). Otherwise stay greedy
        // (deterministic, non-thinking) for plain text.
        let sampler = if req.tools.is_empty() {
            LlamaSampler::chain_simple([LlamaSampler::greedy()])
        } else {
            let schema = build_schema_from_tools(&req.tools);
            let grammar = llama_cpp_2::json_schema_to_grammar(&schema).map_err(|e| {
                PiiError::Internal(format!("llamacpp: schema_to_grammar: {e}"))
            })?;
            LlamaSampler::chain_simple([LlamaSampler::grammar(
                &self.model,
                &grammar,
                "function_schema",
            )
            .map_err(|e| PiiError::Internal(format!("llamacpp: grammar: {e}")))?])
        };
        let mut sampler = sampler;
```

(Keep the rest of `run` unchanged — the `loop` still calls `sampler.sample` /
`sampler.accept`.)

### R3 — parse tool_calls from the output

In `run`, REPLACE the `Ok(ChatResponse { ... })` return (lines ~141-147):

```rust
        Ok(ChatResponse {
            text,
            tool_calls: Vec::new(),
            tokens_in: tokens.len() as u64,
            tokens_out,
            micro_usd: 0,
        })
```

WITH:

```rust
        let tool_calls = if req.tools.is_empty() {
            Vec::new()
        } else {
            parse_tool_calls(&text)
        };
        Ok(ChatResponse {
            text,
            tool_calls,
            tokens_in: tokens.len() as u64,
            tokens_out,
            micro_usd: 0,
        })
```

### R4 — add ONE test (T4: empty_tools_keeps_text_path)

Do NOT add a real-model test (N4: no 2.2GB load). Add only this pure-logic
test to the existing `mod tests` in `llamacpp.rs` (append inside the module):

```rust
    /// R3: a request with no tools must not build a grammar; the pure prompt
    /// builder still renders a wire-format transcript.
    #[test]
    fn empty_tools_keeps_text_path() {
        let msgs = vec![
            ChatMessage::new(Role::System, "sys"),
            ChatMessage::new(Role::User, "hi"),
        ];
        let p = build_prompt(&msgs);
        assert!(p.contains("<|start_of_role|>assistant<|end_of_role|>"));
        assert!(!p.contains("tool"), "no tool scaffolding in text path: {p}");
    }
```

The `tests` module already uses `use super::granite::build_prompt;` — confirm
`build_prompt` and `ChatMessage`/`Role` are in scope; if `Role` is not already
imported in tests, add `use crate::Role;` inside the test module.

## Files to Modify
- EDIT `crates/pii-model/src/llamacpp.rs` — import (R1), sampler branch (R2),
  tool_calls parse (R3), one test (R4). ONLY this file.

## Exit Criteria
- [ ] `cargo test -p pii-model --lib empty_tools_keeps_text_path 2>&1 | grep -qE "test result: ok\. [1-9]"`
- [ ] `cargo test -p pii-model --lib 2>&1 | grep -qE "test result: ok\. [1-9]"` and NOT `failed`
- [ ] `cargo clippy -p pii-model --all-targets -- -D warnings 2>&1 | grep -Ec "^error"` equals 0

## Guardrails (binding)
- Do NOT rewrite `llamacpp.rs` wholesale. Make ONLY the four surgical edits
  (R1 import, R2 sampler, R3 return, R4 test). Match the existing code exactly.
- Do NOT touch `llamacpp_tools.rs`, `specs/`, or any other file.
- Do NOT commit. Do NOT run pidag. Do NOT start background processes.
- Do NOT load a real model in a test (N4).
- Keep `#![forbid(unsafe_code)]` and do NOT add `unsafe`.
- Run `cargo test -p pii-model --lib` and `cargo clippy -p pii-model --all-targets`
  yourself to verify; report the raw `test result:` lines.

## Sizing note
This is an EDIT-EXISTING-FILE micro-task (the risky kind). Do the minimum:
apply each of the 4 edits verbatim from the brief, verify, stop. Do NOT run
clippy cleanups or re-read unrelated parts. If any edit cannot be applied
precisely, leave the file compiling and report which edit failed.
