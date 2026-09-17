# pi-improved — spec-49: split `pii-graph/src/run.rs` into `run/{mod,resume,engine}.rs`

- **Project**: `/projects/pi-improved`
- **Priority**: LOW — Phase 5.2, remainder of F-12 (warn-band paydown, NOT
  a hard failure). `pii-graph/src/run.rs` is 411 effective lines, in the
  380-450 warn band. Cosmetic debt reduction, not a blocker.

## Overview

`crates/pii-graph/src/run.rs` (577 total lines) has three public entry
points — `run`, `run_with_validate`, `resume` — plus a private `Engine`
struct and its `impl Engine` block (the scheduler tick loop). This is a
**pure move-and-re-export refactor** — same discipline as specs 43-45/48:
no behavior change, no test deletions/weakenings, every `pub` item keeps
its exact signature and stays reachable at `pii_graph::run::<name>`
exactly as today.

Ground truth already confirmed by the orchestrator (verify yourself
before starting, do not trust blindly):
- `pub fn run` (line 45), `pub fn run_with_validate` (line 68), `pub fn
  resume` (line 133) are the three public entry points. `run` is a thin
  wrapper calling `run_with_validate(..., true)`.
- `struct Engine<'a>` (line 231) and `impl Engine<'_>` (line 247) hold
  `fn tick_loop` (252, has `#[allow(clippy::too_many_lines)]` and an
  explicit author comment: "Kept as one function for reviewability of
  the batch drain and its two terminal arms; splitting would cost more
  than the line budget saves" — DO NOT split `tick_loop` itself, move it
  whole), plus small helpers `fn metric` (526), `fn cancelled` (537),
  `fn unsatisfiable` (542), `fn finish` (553).
- `Engine` and all its fields are private (no `pub`); confirmed via grep
  that `Engine` has ZERO usages outside this file — safe to make it
  `pub(super)` (visible only within `run/`) without any external impact.
- Both `run_with_validate` (line ~107) and `resume` (line ~209)
  construct an `Engine { ... }` struct literal and call
  `engine.tick_loop(...)` — this is the coupling point requiring `Engine`
  and `tick_loop` to be `pub(super)`, not fully private, once split
  across files.
- No `#[cfg(test)]` module exists in this file (verify with grep before
  starting — if found, this spec's ground truth missed it, move it
  verbatim to `run/tests.rs` per the established precedent from spec-43).

## Requirements

### Functional

- R1. Convert `crates/pii-graph/src/run.rs` into a directory module
  `crates/pii-graph/src/run/` with `mod.rs`, `resume.rs`, `engine.rs`.
  `crates/pii-graph/src/lib.rs`'s existing `mod run;`/`pub mod run;`
  declaration is unchanged — Rust resolves it to `run/mod.rs`
  automatically.
- R2. `run/mod.rs` keeps: the module-level doc comment (lines 1-14), all
  `use` statements, `pub fn run`, `pub fn run_with_validate`, and a
  `mod engine; mod resume;` plus `use engine::Engine;` (or equivalent) so
  `run_with_validate` can still construct `Engine { ... }` and call
  `.tick_loop(...)`.
- R3. `run/engine.rs` holds `struct Engine<'a>` and `impl Engine<'_> {
  fn tick_loop, fn metric, fn cancelled, fn unsatisfiable, fn finish }`
  verbatim. Change `struct Engine` to `pub(super) struct Engine` and
  every one of its fields to `pub(super)` (they need to stay constructible
  via struct-literal syntax from `mod.rs` and `resume.rs`); change `fn
  tick_loop` to `pub(super) fn tick_loop` (the only method called from
  outside `engine.rs`) — leave `metric`/`cancelled`/`unsatisfiable`/
  `finish` as private `fn` (they're only called from within `tick_loop`
  itself, in the same file).
- R4. `run/resume.rs` holds `pub fn resume` verbatim, with a `use
  super::engine::Engine;` (or equivalent path) to construct `Engine {
  ... }` and call `.tick_loop(...)` exactly as today. Re-export `resume`
  from `mod.rs` (`pub use resume::resume;` or `pub mod resume;` with the
  fn already `pub` — pick whichever keeps `pii_graph::run::resume`
  resolving identically; verify with a grep of every external call site
  before finalizing).
- R5. No public API surface changes: `pii_graph::run::run`,
  `pii_graph::run::run_with_validate`, `pii_graph::run::resume` all
  resolve identically after the split. Grep every external call site of
  these three functions workspace-wide before finishing and confirm each
  one still compiles unchanged.

### Non-Functional

- N1. Each new file individually satisfies `xtask lint-files` (≤450
  effective lines, ideally under 380).
- N2. `#![forbid(unsafe_code)]` unaffected.
- N3. No new `unwrap()/expect()/panic!()/todo!()/unimplemented!()`.
- N4. Do NOT split `tick_loop`'s body — move the whole function intact,
  honoring the existing author comment against splitting it further.

## Architecture

```
crates/pii-graph/src/
├── run.rs                    (DELETED — becomes run/)
└── run/
    ├── mod.rs        — module doc comment, run(), run_with_validate(),
    │                    re-exports resume, wires up Engine
    ├── engine.rs      — pub(super) struct Engine, impl Engine {
    │                    tick_loop (pub(super)), metric, cancelled,
    │                    unsatisfiable, finish }
    └── resume.rs      — pub fn resume()
```

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| (existing `pii-graph` integration tests exercising `run`/`resume`, e.g. under `crates/pii-graph/tests/`) | — | still pass, byte-for-byte same assertions — this file itself has no inline tests per ground truth |

## Exit Criteria

- [ ] `[ ! -f crates/pii-graph/src/run.rs ]`
- [ ] `[ -f crates/pii-graph/src/run/mod.rs ]`
- [ ] `[ -f crates/pii-graph/src/run/engine.rs ]`
- [ ] `[ -f crates/pii-graph/src/run/resume.rs ]`
- [ ] `cargo test --workspace --lib --bins --tests`
- [ ] `cargo test --workspace --doc`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo fmt --all --check`
- [ ] `cargo run -q -p xtask -- silent-noop-check`
- [ ] `cargo run -q -p xtask -- lint-files`
- [ ] `! grep -rE "unwrap\(|expect\(|panic!|todo!|unimplemented!" crates/pii-graph/src/run/mod.rs crates/pii-graph/src/run/engine.rs crates/pii-graph/src/run/resume.rs`
- [ ] `grep -q "clippy::too_many_lines" crates/pii-graph/src/run/engine.rs`

## Guardrails

- G1. Do NOT change any function's body, signature, or behavior — this
  is a pure move. Do NOT split `tick_loop` into smaller functions.
- G2. Do NOT touch `specs/` (read-only for the workhorse).
- G3. Do NOT commit — leave the diff in the working tree for the
  orchestrator to review and commit.
- G4. Do NOT touch `crates/pii-graph/src/lib.rs` unless its `run` module
  declaration or re-export genuinely needs a path change (it should not
  — directory modules resolve transparently).
- G5. If splitting reveals a visibility conflict Rust's module system
  won't allow, STOP and report rather than silently widening visibility
  beyond `pub(super)` — matching every prior split spec's own guardrail.
- G6. No `unsafe` code (`#![forbid(unsafe_code)]`, workspace-wide).
