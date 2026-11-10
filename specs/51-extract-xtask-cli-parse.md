# pi-improved — spec-51: extract xtask/src/main.rs's arg-parsing helpers into cli_parse.rs

- **Project**: `<repo>`
- **Priority**: LOW — Phase 5.2, final F-12 warn-band item.
  `xtask/src/main.rs` is 411 effective lines, in the 380-450 warn band.
  Cosmetic debt reduction, not a blocker — this closes out F-12's
  optional warn-band ratchet entirely (last of 6 files from the original
  026 audit; agent.rs, silent_noop.rs, service.rs, mem.rs, run.rs, and
  pii-tui/lib.rs are already done via specs 43-50).

## Overview

`xtask/src/main.rs` is a binary crate root mixing CLI subcommand
dispatch (`main`), gate-running helpers (`report`, `run_cmd`,
`workspace_root`, `run_ci`), four self-contained argv-parsing functions
(`parse_mem_eval`, `parse_worktree_flag`, `parse_promote_collect`,
`parse_rss_gate`), and two test blocks. This is a **pure move-and-re-export
refactor** — same discipline as specs 43-45/48-50: no behavior change,
no test deletions/weakenings, `main()` itself keeps its exact CLI surface
(every subcommand, every flag, every error message byte-identical).

`xtask` is a **binary crate** (`main.rs` is the crate root, like
`pii-tui`'s `lib.rs` in spec-50, not a `mod.rs` inside a directory
module) — this split creates a SIBLING file next to `main.rs`, declared
via a `mod` statement, not a subdirectory restructuring of `main.rs`
itself.

Ground truth already confirmed by the orchestrator (verify yourself
before starting, do not trust blindly):
- `main.rs` has TWO separate test constructs: a `#[cfg(test)] mod
  top_level_tests { ... }` block starting at line 312 (uses
  `#![allow(clippy::unwrap_used, ...)]` and `#![allow(clippy::indexing_slicing)]`
  inner attributes, imports `crate::memory_eval` and `crate::net_check`),
  AND a separate bare `#[cfg(test)] #[test] fn deps_check_rejects_redb_outside_store`
  OUTSIDE that mod block (comment explicitly says "top-level so `-- --exact`
  matches the bare path" — this is deliberate, not an accident, do NOT move
  it into `top_level_tests` or anywhere else; leave both test constructs
  exactly where they are in `main.rs`, untouched by this spec).
- **This spec does NOT touch `main()`, `run_ci`, `report`, `run_cmd`,
  `workspace_root`, or either test construct.** It ONLY extracts the four
  pure argv-parsing functions, because they are the only clean,
  zero-risk cut: `fn parse_mem_eval(rest: &[String]) -> Result<(String,
  String, usize), String>` (line 231), `fn parse_worktree_flag(rest:
  &[String]) -> Option<std::path::PathBuf>` (264), `fn
  parse_promote_collect(...)` (279), `fn parse_rss_gate(rest: &[String])
  -> Result<(u64, u64, u64, u32), String>` (543, AFTER the test module —
  verify its exact current line by re-reading the file, do not assume
  line numbers are stable after any earlier edit).
- Each of the four `parse_*` functions is called exactly once, from
  inside `main()`'s match arms (confirmed via grep: `parse_rss_gate` at
  line ~125, `parse_mem_eval` at ~139, `parse_worktree_flag` at ~173,
  `parse_promote_collect` at ~182) — they have no other callers anywhere
  in the workspace, no test coverage of their own visible in the file
  (their behavior is only exercised indirectly via `main`'s CLI surface,
  which is unaffected by this spec since it's a pure move).
- All four are currently private (`fn`, no `pub`) — after the move they
  need to be reachable from `main.rs`'s `main()` function, which is a
  sibling module relationship: `pub(crate) fn` in the new file, `use
  crate::cli_parse::{parse_mem_eval, parse_worktree_flag,
  parse_promote_collect, parse_rss_gate};` in `main.rs`.

## Requirements

### Functional

- R1. Create `xtask/src/cli_parse.rs`. Add `mod cli_parse;` to
  `main.rs` (near the other `mod` declarations at the top of the file).
- R2. Move `parse_mem_eval`, `parse_worktree_flag`, `parse_promote_collect`,
  `parse_rss_gate` verbatim into `cli_parse.rs`, changing each from
  private `fn` to `pub(crate) fn` (needed for `main.rs` to call them
  as sibling-module items).
- R3. In `main.rs`, replace the four function definitions with a `use
  crate::cli_parse::{parse_mem_eval, parse_worktree_flag,
  parse_promote_collect, parse_rss_gate};` import, and leave every call
  site (`main()`'s match arms) completely unchanged — the function names
  resolve identically after the `use`.
- R4. Do NOT move, touch, or renumber anything else in `main.rs`:
  `main`, `run_ci`, `report`, `run_cmd`, `workspace_root`, the module
  doc comment, the crate-level `#![forbid(unsafe_code)]` /
  `#![allow(...)]` attributes, `mod top_level_tests { ... }`, or the
  standalone `deps_check_rejects_redb_outside_store` test all stay
  exactly where they are, byte-identical.
- R5. `cli_parse.rs` needs its own `use` statements for whatever the
  four functions reference (check: do they need anything beyond
  `std::path::PathBuf` and the standard library? Read each function body
  before finalizing imports — do not guess).

### Non-Functional

- N1. `main.rs` and `cli_parse.rs` both individually satisfy `xtask
  lint-files` (≤450 effective lines; `main.rs` should drop comfortably
  under 380 after removing ~140 lines of parsing logic).
- N2. `#![forbid(unsafe_code)]` unaffected.
- N3. No new `unwrap()/expect()/panic!()/todo!()/unimplemented!()`.
- N4. `cargo xtask` (the binary) behaves byte-identically for every
  subcommand — this is enforced by the existing `top_level_tests`
  module and the standalone `deps_check_rejects_redb_outside_store`
  test continuing to pass unmodified, plus a manual smoke check
  (`cargo run -q -p xtask -- rss-gate --secs 5` or similar, exercising
  at least one of the four moved parse functions through the real CLI
  path) as part of your own verification before declaring done.

## Architecture

```
xtask/src/
├── main.rs         — mod cli_parse; + use crate::cli_parse::{...};
│                      main(), run_ci, report, run_cmd, workspace_root,
│                      mod top_level_tests, deps_check_rejects_redb_outside_store
│                      (all UNCHANGED)
└── cli_parse.rs     — parse_mem_eval, parse_worktree_flag,
                       parse_promote_collect, parse_rss_gate (moved,
                       pub(crate))
```

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| (existing `top_level_tests` + `deps_check_rejects_redb_outside_store` + `memory_eval_fixture_is_not_rigged`, all untouched) | — | still pass, byte-for-byte same behavior |
| Manual smoke: `cargo run -q -p xtask -- rss-gate --secs 5` (or any subcommand exercising a moved parse fn) | CLI args | same success/error behavior as before the move |

## Exit Criteria

- [ ] `[ -f xtask/src/cli_parse.rs ]`
- [ ] `grep -q "fn parse_mem_eval" xtask/src/cli_parse.rs`
- [ ] `grep -q "fn parse_worktree_flag" xtask/src/cli_parse.rs`
- [ ] `grep -q "fn parse_promote_collect" xtask/src/cli_parse.rs`
- [ ] `grep -q "fn parse_rss_gate" xtask/src/cli_parse.rs`
- [ ] `! grep -q "fn parse_mem_eval" xtask/src/main.rs`
- [ ] `grep -q "mod top_level_tests" xtask/src/main.rs`
- [ ] `grep -q "fn deps_check_rejects_redb_outside_store" xtask/src/main.rs`
- [ ] `cargo test --workspace --lib --bins --tests`
- [ ] `cargo test --workspace --doc`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo fmt --all --check`
- [ ] `cargo run -q -p xtask -- silent-noop-check`
- [ ] `cargo run -q -p xtask -- lint-files`
- [ ] `! grep -rE "unwrap\(|expect\(|panic!|todo!|unimplemented!" xtask/src/cli_parse.rs`

## Guardrails

- G1. Do NOT change any function's body, signature, or behavior — this
  is a pure move of exactly four functions, nothing else.
- G2. Do NOT touch `specs/` (read-only for the workhorse).
- G3. Do NOT commit — leave the diff in the working tree for the
  orchestrator to review and commit.
- G4. Do NOT move, edit, or reorder `main`, `run_ci`, `report`,
  `run_cmd`, `workspace_root`, `mod top_level_tests`, or the standalone
  `deps_check_rejects_redb_outside_store` test — they stay in `main.rs`
  exactly where they are today.
- G5. Do NOT merge the two separate test constructs
  (`top_level_tests` and the standalone top-level test) — they are
  deliberately separate per the existing code comment about `-- --exact`
  path matching; leave that distinction untouched.
- G6. No `unsafe` code (`#![forbid(unsafe_code)]`, workspace-wide).
