# pi-improved — spec-44: split `xtask/src/silent_noop.rs` into `silent_noop/{mod,scans,helpers,tests}.rs`

- **Project**: `<repo>`
- **Priority**: HIGH — `xtask/src/silent_noop.rs` is the ONLY current hard
  `xtask lint-files` failure (458 effective lines, 8 over the 450 cap); every
  other flagged file is in the 380-450 warn band, not a hard failure. Fix
  this before touching the warn-band files.

## Overview

`xtask/src/silent_noop.rs` implements `xtask silent-noop-check` (spec 24):
five independent scans (methods, events, config, graph-keys, protected
paths) that each look for a declared-but-unwired surface, plus shared
report/allowlist plumbing and the public `run()` entry point. This is a
**pure move-and-re-export refactor**: no scan logic changes, no new
findings, no allowlist changes, no test deletions or weakenings.

Ground truth, read before writing anything else: `xtask/src/silent_noop.rs`
in full. Key facts already confirmed:
- `xtask/src/main.rs` declares `mod silent_noop;` (line 24) and calls
  `silent_noop::run(&workspace_root(), ...)` at two call sites (lines 80,
  123) plus asserts on the string `"silent_noop::run"` in a test at line
  399-400 (`top_level_tests::run_ci_includes_self_improvement`) — **that
  assertion greps `main.rs`'s own source text for the literal substring
  `silent_noop::run`, not `silent_noop::mod::run` or any other path, so the
  public entry point's call *spelling* in `main.rs` must stay exactly
  `silent_noop::run(...)`** (Rust resolves `mod silent_noop;` to
  `silent_noop/mod.rs` automatically, so this is unaffected by becoming a
  directory module, but confirm by re-reading that assertion's exact string
  match before finishing).
- The file has one `#[cfg(test)] mod tests { ... }` block starting at line
  572 (129 lines of tests, excluded from the effective-line count already).
- Public/crate-visible items used outside the file: `pub(crate) struct
  AllowEntry`, `pub(crate) fn run`, `pub(crate) fn allowlist_for` — confirm
  the full list by grep, do not assume this list is exhaustive.

## Requirements

### Functional

- R1. Convert `xtask/src/silent_noop.rs` into a directory module
  `xtask/src/silent_noop/` with `mod.rs`, `scans.rs`, `helpers.rs`.
  `xtask/src/main.rs`'s `mod silent_noop;` is unchanged.
- R2. `silent_noop/mod.rs` keeps: the module doc comment (original lines
  1-20), `AllowEntry` struct, the allowlist table + `allowlist()`/
  `allowlist_for()` functions, `pub(crate) fn run()`, `report_lines()`,
  `mod scans; mod helpers;` declarations, and the `#[cfg(test)] mod tests`
  block (moved verbatim — see R5).
- R3. `silent_noop/scans.rs` holds the five scan functions: `scan_methods`,
  `scan_events`, `scan_config`, `scan_graph_keys`, `scan_protected_paths`,
  plus their direct single-purpose helpers that only one scan calls:
  `dispatch_has_arm` (used only by `scan_methods`), `config_field_read`
  (used only by `scan_config`), `graph_declares_key`/`graph_key_read` (used
  only by `scan_graph_keys`), `protected_path_wired` (used only by
  `scan_protected_paths`), `is_filter_or_render_site` (used only by
  `scan_events`). Moved verbatim except `use` statements and visibility
  (`pub(super)` for the 5 scan entry points `mod.rs`'s `run()` calls,
  private for the rest unless a cross-module call requires otherwise).
- R4. `silent_noop/helpers.rs` holds the small ID/parsing utilities shared
  across multiple scans: `method_id`, `event_id`, `config_id`,
  `graph_key_id`, `enum_variant_names`, `variant_name_of`,
  `collect_rust_files`, `collect_crates_files`, `require_written_reasons`.
  Moved verbatim, `pub(super)` where `scans.rs` needs them.
- R5. Tests are **not split, not rewritten, not deleted**. The entire
  `#[cfg(test)] mod tests { ... }` block moves as one unit — either staying
  inline in `mod.rs` (if that keeps `mod.rs` comfortably under the 380-line
  warn band, since test bodies are excluded from the effective count
  entirely) or into its own `silent_noop/tests.rs` (following the exact
  precedent set by spec-43's `agent/tests.rs`: an inner `#![cfg(test)]`
  attribute at the top of the file, included via `mod tests;` in `mod.rs`
  — the `xtask/src/lint_files.rs` support for this shape already landed in
  spec-43, do not re-implement it). Test *content* must be byte-for-byte
  identical; only import paths may change if items moved to sibling
  modules (e.g. `use super::*;` adjustments).
- R6. No public API surface changes: `xtask::silent_noop::run` (called from
  `main.rs`) and the exact string `silent_noop::run(` must still appear
  verbatim in `main.rs`'s source (re-verify the R2 exit criterion below
  actually re-checks this, don't just trust the file compiles).

### Non-Functional

- N1. Each new file individually satisfies `xtask lint-files` (≤450
  effective lines).
- N2. `#![forbid(unsafe_code)]` unaffected — no `unsafe` in the original
  file, none introduced.
- N3. No new `unwrap()/expect()/panic!()/todo!()/unimplemented!()` outside
  `#[cfg(test)]` bodies.

## Architecture

```
xtask/src/
├── silent_noop.rs               (DELETED — becomes silent_noop/)
└── silent_noop/
    ├── mod.rs        — AllowEntry, allowlist()/allowlist_for(), run(),
    │                    report_lines(), `mod scans; mod helpers;`,
    │                    tests (inline or `mod tests;` -> tests.rs)
    ├── scans.rs       — scan_methods, scan_events, scan_config,
    │                    scan_graph_keys, scan_protected_paths, and their
    │                    single-purpose helpers
    └── helpers.rs     — method_id/event_id/config_id/graph_key_id,
                          enum_variant_names/variant_name_of,
                          collect_rust_files/collect_crates_files,
                          require_written_reasons
```

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| (all existing tests in `silent_noop.rs`'s `mod tests`, unchanged) | — | every test that passed before the move still passes after, byte-for-byte same assertions |
| `run_ci_includes_self_improvement` (existing, `xtask/src/main.rs`) | grep `main.rs`'s own source for `"silent_noop::run"` | still passes — the call spelling in `main.rs` is unaffected by `silent_noop.rs` becoming a directory module |

## Exit Criteria

- [ ] `[ ! -f xtask/src/silent_noop.rs ]`
- [ ] `[ -f xtask/src/silent_noop/mod.rs ]`
- [ ] `[ -f xtask/src/silent_noop/scans.rs ]`
- [ ] `[ -f xtask/src/silent_noop/helpers.rs ]`
- [ ] `cargo test --workspace --lib --bins --tests`
- [ ] `cargo test --workspace --doc`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo fmt --all --check`
- [ ] `cargo run -q -p xtask -- silent-noop-check`
- [ ] `cargo run -q -p xtask -- lint-files`
- [ ] `grep -q "silent_noop::run(" xtask/src/main.rs`
- [ ] `! grep -rE "unwrap\(|expect\(|panic!|todo!|unimplemented!" xtask/src/silent_noop/scans.rs xtask/src/silent_noop/helpers.rs`
- [ ] `! sed '/^mod tests {$/,/^}$/d; /^#!\[cfg(test)\]$/,$d' xtask/src/silent_noop/mod.rs | grep -qE "unwrap\(|expect\(|panic!|todo!|unimplemented!"`

## Guardrails

- G1. Do NOT change any test's assertions, names, or add/remove any test.
- G2. Do NOT change scan logic, allowlist entries, or reported violation
  strings — this must produce byte-identical `silent-noop-check --list`
  output before and after (diff it yourself before declaring done).
- G3. Do NOT touch `specs/` (read-only for the workhorse).
- G4. Do NOT commit — leave the diff in the working tree for the
  orchestrator to review and commit.
- G5. Do NOT touch `main.rs` unless the `mod silent_noop;` declaration
  genuinely needs no change (it shouldn't — confirm, don't guess) and no
  other file outside `xtask/src/silent_noop.rs`/`silent_noop/`.
- G6. `xtask lint-files`'s own hard exit-criterion check (`cargo run -q -p
  xtask -- lint-files`) must exit 0 — it currently fails (exit 1) solely
  because of this file; after the split it must pass cleanly with no
  remaining hard violation. Do not touch any *other* warn-band file
  (`pii-store/src/mem.rs`, `pii-graph/src/run.rs`, `pii-net/src/
  service.rs`, `pii-tui/src/lib.rs`, `xtask/src/main.rs`) — those are
  separate, future work items, out of scope for this spec.
