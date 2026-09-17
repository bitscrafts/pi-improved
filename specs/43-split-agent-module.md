# pi-improved — spec-43: split `agent.rs` into `agent/{mod,turn,tools,memorize}.rs`

- **Project**: `/projects/pi-improved`
- **Priority**: HIGH — first item of Phase 5.2 (structural debt ratchet), per
  the 026 report's own sequencing (deferred until Phase 5.1 Track A finished,
  which it now has — specs 37-42 all landed and gate-green).

## Overview

`crates/pii-server/src/agent.rs` is 1329 raw lines and **446 *effective*
lines** (`xtask lint-files`'s own non-blank/non-comment/non-`#[cfg(test)]`
count) — 66 lines under the workspace's hard 450-line gate, but already past
the 380-line warn threshold and the single largest file in the crate. Any
further work on the agent node (Track A left this file with a `fail` port,
`route_provider_failure`, and `FailurePayload` classification already living
here) will push it over the hard cap.

This spec is a **pure move-and-re-export refactor**: no behavior changes, no
new functionality, no test deletions or weakenings. Every `pub`/`pub(crate)`
item keeps its exact signature; `crate::agent::AgentFactory` (the only
external call site — `registry.rs`, 2 call sites) keeps resolving to the
same type at the same path.

## Requirements

### Functional

- R1. Convert `crates/pii-server/src/agent.rs` into a directory module
  `crates/pii-server/src/agent/` with `mod.rs`, `turn.rs`, `tools.rs`,
  `memorize.rs`. `crates/pii-server/src/lib.rs`'s `pub mod agent;` is
  unchanged (Rust resolves it to `agent/mod.rs` automatically).
- R2. `agent/mod.rs` keeps: the module doc comment (lines 1-5 of the
  original file), `AgentNode` struct, `AgentFactory` struct,
  `resolve_model_for_test`/`resolve_model`, `route_provider_failure`, the
  `impl pii_graph::node::NodeFactory for AgentFactory` block (kind/effects/
  required_caps/ports/build), `mod turn; mod tools; mod memorize;`
  declarations, and the `#[cfg(test)] mod tests { ... }` block (moved
  verbatim, unchanged — see R6).
- R3. `agent/turn.rs` holds `impl GraphNode for AgentNode { fn run(...) }` —
  the provider/tool turn loop (original lines ~199-414) — moved verbatim
  except for `use` statements needed at the new location. `use
  super::{AgentNode, route_provider_failure};` plus whatever
  `super::memorize::{split_memorize, history_messages}` and
  `super::tools` items it calls.
- R4. `agent/tools.rs` holds the `impl AgentNode { fn emit_live, fn
  run_tool_calls, fn call_tool_bounded }` block (original lines ~463-589),
  moved verbatim except for `use super::AgentNode;`.
- R5. `agent/memorize.rs` holds `split_memorize` and `history_messages`
  (original lines ~416-461), moved verbatim, `pub(super)` (or `pub(crate)`
  if `turn.rs` needs cross-module visibility — verify against Rust's
  actual privacy rules for sibling modules, do not guess).
- R6. Tests are **not split, not rewritten, not deleted**. Every test in
  the original `#[cfg(test)] mod tests { ... }` block moves as one unit
  into `agent/mod.rs` (or, if that alone would still put `mod.rs` over the
  380-line warn band on *effective* lines — verify with `xtask lint-files`
  after the move, since test bodies are excluded from the effective count
  entirely per `count_effective_lines`'s own `#[cfg(test)]` exclusion —
  the test module may instead live in its own `agent/tests.rs` included via
  `#[cfg(test)] mod tests;` from `mod.rs`; either placement is acceptable,
  but the test *content* must be byte-for-byte identical, only import paths
  may change (e.g. `use super::*;` → adjusted paths if items moved to
  sibling modules).
- R7. No public API surface changes: `pii_server::agent::AgentFactory`
  remains constructible exactly as `registry.rs`'s two call sites do it
  today (do not run `cargo build` and declare done — grep `registry.rs`
  for `agent::AgentFactory {` and confirm the field list matches the
  original `AgentFactory` struct's fields with no additions/removals).

### Non-Functional

- N1. Each new file individually satisfies `xtask lint-files` (≤450
  effective lines; ideally under the 380 warn band, but the hard cap is
  the actual gate).
- N2. `#![forbid(unsafe_code)]` (workspace-wide, `Cargo.toml:54`) is
  unaffected — no `unsafe` existed in the original file and none may be
  introduced.
- N3. No new `unwrap()/expect()/panic!()/todo!()/unimplemented!()` outside
  `#[cfg(test)]` bodies (the original file already has zero in production
  code — do not introduce any while moving code around).

## Architecture

```
crates/pii-server/src/
├── agent.rs                    (DELETED — becomes agent/)
└── agent/
    ├── mod.rs        — AgentNode, AgentFactory, NodeFactory impl,
    │                    resolve_model(_for_test), route_provider_failure,
    │                    `mod turn; mod tools; mod memorize;`,
    │                    #[cfg(test)] mod tests (or `mod tests;` -> tests.rs)
    ├── turn.rs        — impl GraphNode for AgentNode { fn run }
    ├── tools.rs       — impl AgentNode { emit_live, run_tool_calls,
    │                    call_tool_bounded }
    └── memorize.rs    — split_memorize, history_messages
```

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| (all existing tests in `agent.rs`'s `mod tests`, unchanged) | — | every test that passed before the move still passes after, byte-for-byte same assertions |
| `agent_module_has_no_single_file_over_450_lines` (new, in `xtask` or a `pii-server` test) | run `xtask lint-files` over `crates/pii-server/src/agent/*.rs` | zero files in the `agent/` directory exceed 450 effective lines |

## Exit Criteria

- [ ] `[ ! -f crates/pii-server/src/agent.rs ]`
- [ ] `[ -f crates/pii-server/src/agent/mod.rs ]`
- [ ] `[ -f crates/pii-server/src/agent/turn.rs ]`
- [ ] `[ -f crates/pii-server/src/agent/tools.rs ]`
- [ ] `[ -f crates/pii-server/src/agent/memorize.rs ]`
- [ ] `cargo test --workspace --lib --bins --tests`
- [ ] `cargo test --workspace --doc`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo fmt --all --check`
- [ ] `cargo run -q -p xtask -- silent-noop-check`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | grep -q "agent/mod.rs" && echo NEVER_HARD_FAIL || true`
- [ ] `! grep -rE "unwrap\(|expect\(|panic!|todo!|unimplemented!" crates/pii-server/src/agent/mod.rs crates/pii-server/src/agent/turn.rs crates/pii-server/src/agent/tools.rs crates/pii-server/src/agent/memorize.rs`

## Guardrails

- G1. Do NOT change any test's assertions, names, or add/remove any test.
  This is a move, not a rewrite.
- G2. Do NOT change `AgentNode`'s or `AgentFactory`'s field names, types,
  or visibility (`pub`/`pub(crate)`) — external and test code depends on
  the exact current shape.
- G3. Do NOT touch `specs/` (read-only for the workhorse).
- G4. Do NOT commit — leave the diff in the working tree for the
  orchestrator to review and commit.
- G5. Do NOT touch `registry.rs` or any other file outside
  `crates/pii-server/src/agent.rs`/`agent/` unless a genuinely necessary
  import path changes (report any such change explicitly — it should not
  be needed since the module's external path `crate::agent::AgentFactory`
  is unchanged).
- G6. If splitting reveals a genuine coupling problem (e.g. a private
  field needed across modules that Rust's visibility rules won't allow
  without widening it beyond `pub(crate)`), STOP and report it rather than
  silently widening visibility further than necessary — `pub(super)` is
  preferred over `pub(crate)` where it suffices.
