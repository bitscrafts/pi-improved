# pi-improved — spec-50: split `pii-tui/src/lib.rs` into `{state,render,input,driver}.rs` + `lib.rs`

- **Project**: `<repo>`
- **Priority**: LOW — Phase 5.2, remainder of F-12 (warn-band paydown, NOT
  a hard failure). `pii-tui/src/lib.rs` is 435 effective lines, in the
  380-450 warn band. Cosmetic debt reduction, not a blocker.

## Overview

`crates/pii-tui/src/lib.rs` (679 total lines) is a single crate-root file
mixing four concerns: protocol-derived state/types, pure rendering,
keyboard-input handling + session init, and the async driver loop. This
is a **pure move-and-re-export refactor** — same discipline as specs
43-45/48/49: no behavior change, no test deletions/weakenings, every
`pub` item keeps its exact signature and stays reachable at its current
`pii_tui::<name>` path exactly as today.

`pii-tui` is a **library crate** (`lib.rs` is the crate root, not a
`mod.rs` inside a directory module like the previous specs' targets) —
this split creates SIBLING files next to `lib.rs`, declared via `mod`
statements in `lib.rs`, not a `lib/` subdirectory.

Ground truth already confirmed by the orchestrator (verify yourself
before starting, do not trust blindly) — function/type map in file order:
- Lines 1-6: crate-level doc comment (`//!` — "a pure protocol client",
  "holds no domain state"). Stays in `lib.rs`.
- `pub enum TuiError` (22), `pub enum ConnStatus` (33), `pub enum
  LogLine` (51), `pub struct AppState` (80), `impl Default for AppState`
  (94), `impl AppState { ... }` (105, includes `push_event`,
  `tick_thinking`, and other state-mutation methods — read the file to
  get the exact method list) — this is the **state** concern.
- `pub fn render` (187), `fn render_log_line` (256, private), `fn
  render_event_line` (291, private) — this is the **render** concern.
- `enum KeyOutcome` (407, private), `fn handle_slash_command` (424,
  private), `async fn handle_key` (452, private), `async fn
  init_session` (500, private) — this is the **input** concern.
- `pub async fn run_tui` (551), `fn _reconnect_hint` (616, private,
  `#[allow(dead_code)]`) — this is the **driver** concern, and it calls
  into `render`, `handle_key`, and `init_session` from the other three
  files, so it must be able to see them (via `pub(crate)` or a `use`
  of the sibling modules — Rust modules within one crate can already see
  each other's `pub(crate)` items without any visibility widening beyond
  crate-internal).
- `#[cfg(test)] mod tests { ... }` at line 620 (60 lines) — uses `super::{AppState, KeyOutcome, handle_slash_command}` per grep; move
  verbatim, update its `super::` imports to reference the new module
  paths (e.g. `crate::state::AppState`, `crate::input::{KeyOutcome,
  handle_slash_command}`).
- Only external usage of this crate (checked workspace-wide): `pii-cli`'s
  `cmd_tui.rs` calls `pii_tui::run_tui(r, w).await` — nothing else is
  imported from `pii_tui` externally. `AppState`, `LogLine`, `ConnStatus`,
  `TuiError`, `render` are all currently `pub` but have ZERO external
  callers outside this crate's own tests — confirm this yourself with a
  workspace-wide grep before deciding final visibility, but the spec's
  assumption is they may need to STAY `pub` (re-exported from `lib.rs`)
  since a library crate's whole point is its public surface, even if
  nothing in this workspace happens to use them from outside yet.

## Requirements

### Functional

- R1. Create four new sibling files next to `lib.rs`:
  `crates/pii-tui/src/state.rs`, `render.rs`, `input.rs`, `driver.rs`.
  Add `mod state; mod render; mod input; mod driver;` to `lib.rs`.
- R2. `state.rs` holds `TuiError`, `ConnStatus`, `LogLine`, `AppState`,
  `impl Default for AppState`, `impl AppState { ... }` verbatim.
- R3. `render.rs` holds `render`, `render_log_line`, `render_event_line`
  verbatim, with `use crate::state::{AppState, LogLine};` (and `Event`
  from `pii_proto` as today) to reach the types it renders.
- R4. `input.rs` holds `KeyOutcome`, `handle_slash_command`, `handle_key`,
  `init_session` verbatim, with `use crate::state::AppState;` (and
  whatever else it needs from `pii_proto`/`crossterm`).
- R5. `driver.rs` holds `run_tui`, `_reconnect_hint` verbatim, with `use
  crate::render::render; use crate::input::{handle_key, init_session,
  KeyOutcome}; use crate::state::AppState;` — wire up whichever items
  from `state`/`render`/`input` need to be `pub(crate)` (not fully
  `pub`) for `driver.rs` to reach them, since they're crate-internal
  wiring, not part of the crate's own public API.
- R6. `lib.rs` keeps: the crate-level doc comment, the four `mod`
  declarations, and re-exports whatever was previously directly
  reachable at `pii_tui::<name>` (i.e. `pub use state::{TuiError,
  ConnStatus, LogLine, AppState}; pub use render::render; pub use
  driver::run_tui;` — or leave `TuiError`/`ConnStatus`/`LogLine`/
  `AppState`/`render` as `pub(crate)`-only if a workspace-wide grep
  confirms zero external callers depend on their current full `pub`
  reachability; `run_tui` MUST stay reachable at exactly
  `pii_tui::run_tui` since `pii-cli`'s `cmd_tui.rs` depends on it).
- R7. Move the `#[cfg(test)] mod tests` block into `driver.rs` (it tests
  `AppState`/`KeyOutcome`/`handle_slash_command`, which is fine to test
  from any of the split files — put it wherever is least awkward,
  `driver.rs` or `input.rs`, your call) with its `super::` imports
  rewritten to the correct new paths.
- R8. No public API surface changes for `pii-cli`'s call site: `grep -n
  "pii_tui::" crates/pii-cli/src/cmd_tui.rs` must show the exact same
  call after the split.

### Non-Functional

- N1. Each new file individually satisfies `xtask lint-files` (≤450
  effective lines, ideally under 380).
- N2. `#![forbid(unsafe_code)]` unaffected (crate-level attribute stays
  in `lib.rs` if present — verify).
- N3. No new `unwrap()/expect()/panic!()/todo!()/unimplemented!()`
  outside the moved test module (which already has
  `#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]`
  per the original file — carry that allow attribute with the test
  module wherever it lands).

## Architecture

```
crates/pii-tui/src/
├── lib.rs        — crate doc comment, mod declarations, pub re-exports
├── state.rs       — TuiError, ConnStatus, LogLine, AppState + impls
├── render.rs      — render, render_log_line, render_event_line
├── input.rs       — KeyOutcome, handle_slash_command, handle_key,
│                     init_session
└── driver.rs       — run_tui, _reconnect_hint, #[cfg(test)] mod tests
```

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| (the existing `#[cfg(test)] mod tests` block, moved verbatim with updated imports — do not rename or weaken any assertion) | — | all still pass, byte-for-byte same behavior |

## Exit Criteria

- [ ] `[ -f crates/pii-tui/src/state.rs ]`
- [ ] `[ -f crates/pii-tui/src/render.rs ]`
- [ ] `[ -f crates/pii-tui/src/input.rs ]`
- [ ] `[ -f crates/pii-tui/src/driver.rs ]`
- [ ] `grep -q "pii_tui::run_tui" crates/pii-cli/src/cmd_tui.rs`
- [ ] `cargo test --workspace --lib --bins --tests`
- [ ] `cargo test --workspace --doc`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo fmt --all --check`
- [ ] `cargo run -q -p xtask -- silent-noop-check`
- [ ] `cargo run -q -p xtask -- lint-files`
- [ ] `! grep -rE "unwrap\(|expect\(|panic!|todo!|unimplemented!" crates/pii-tui/src/lib.rs crates/pii-tui/src/state.rs crates/pii-tui/src/render.rs crates/pii-tui/src/input.rs`

## Guardrails

- G1. Do NOT change any function's body, signature, or behavior — this
  is a pure move.
- G2. Do NOT touch `specs/` (read-only for the workhorse).
- G3. Do NOT commit — leave the diff in the working tree for the
  orchestrator to review and commit.
- G4. Do NOT touch `crates/pii-cli/src/cmd_tui.rs` — its call site must
  keep working unchanged, not be edited to match a new internal path.
- G5. If a visibility conflict arises reorganizing crate-internal items,
  prefer `pub(crate)` over full `pub` unless R6 explicitly requires
  crate-external reachability — matching every prior split spec's own
  guardrail.
- G6. No `unsafe` code (`#![forbid(unsafe_code)]`, workspace-wide).
