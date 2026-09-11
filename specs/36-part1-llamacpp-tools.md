# Spec: spec-36 part1 — llamacpp_tools.rs helper functions

**Project**: /projects/pi-improved
**Phase**: 4 - spec 36 (in-process llama-cpp tool calls), micro-task part 1

## Overview

Implement TWO isolated helper functions in a NEW file
`crates/pii-model/src/llamacpp_tools.rs`, and wire the module into `lib.rs`.
This is part 1 of a chain: it only creates the helpers + their unit tests. It
does NOT touch `llamacpp.rs` or modify the decode loop (that is a later part).

## Requirements

### Functional
- R1. `pub fn build_schema_from_tools(tools: &[ToolSpec]) -> String` — build a
  JSON string from a list of tool specs. Each tool contributes its `name`,
  `description`, and `parameters` (the tool's `schema` field) into a JSON
  object; return the whole object's JSON serialization.
- R2. `pub fn parse_tool_calls(output: &str) -> Vec<ToolCall>` — parse the
  model's JSON output into a `Vec<ToolCall>`. Accept BOTH shapes:
  `{"name":"x","arguments":{...}}` and
  `{"function":{"name":"x","arguments":{...}}}`. For `arguments`, if the JSON
  value is a string, parse it as JSON; otherwise use it as-is. On any parse
  failure, return an empty vec.
- R3. The file compiles standalone and the module is declared in lib.rs as
  `pub mod llamacpp_tools;`.

### Non-Functional
- Uses the existing `ToolSpec` and `ToolCall` types from `crate::` scope —
  do NOT redefine them. Uses `serde_json`.
- No unsafe. No new dependencies beyond what pii-model already has
  (`serde_json` is present).

## TDD Contract

| Test Name | Input | Expected Output |
|-----------|-------|-----------------|
| `schema_includes_tool_names` | one ToolSpec `{name:"fs_read", description:"r", schema:{type:"object"}}` | returned string parses to JSON containing `"fs_read"` |
| `parse_plain_shape` | `{"name":"fs_read","arguments":{"path":"/x"}}` | one ToolCall, name `fs_read`, arguments has `path` |
| `parse_wrapper_shape` | `{"function":{"name":"fs_read","arguments":{"path":"/x"}}}` | one ToolCall, name `fs_read` |
| `parse_invalid_returns_empty` | `"not json"` | `vec![]` |

## Files to Modify
- CREATE `crates/pii-model/src/llamacpp_tools.rs`
- EDIT `crates/pii-model/src/lib.rs` (add `pub mod llamacpp_tools;` beside the
  other module declarations — read lib.rs once to place it correctly)

## Exit Criteria
- [ ] `cargo test -p pii-model --lib llamacpp_tools 2>&1 | grep -qE "test result: ok\. [1-9]"`

## Guardrails
- Do NOT edit or read `crates/pii-model/src/llamacpp.rs` (that is a later part).
- Do NOT touch `specs/`. Do NOT commit (orchestrator-only).
- Do NOT load a real model in any test.
- Do NOT start background processes or run pidag.
- If you cannot finish, leave a partial but COMPILING file.
