# pi-improved — spec-56: pii-tools test coverage for bash/fs/worktree (F-19)

- **Project**: `<repo>`
- **Priority**: MEDIUM — Phase 5.3, F-19 remainder. `pii-tools` has 4/7
  source files with inline `#[cfg(test)]` coverage already; the
  remaining three (`bash.rs`, `fs.rs`, `worktree.rs`) have zero. No
  `crates/pii-tools/tests/` integration-test directory exists at all —
  verify with `ls crates/pii-tools/tests/ 2>/dev/null` before starting.

## Overview

Add inline `#[cfg(test)] mod tests` blocks to each of the three files
(matching this crate's own existing convention — the other 4 files
already tested in `pii-tools` use inline modules, not a separate
`tests/` integration dir; follow that pattern here rather than
introducing a second convention in the same crate).

Ground truth already confirmed by the orchestrator (verify field names/
exact behavior yourself from the actual source before writing tests —
this is a starting map, not a substitute for reading the files):

- `bash.rs` (144 lines): `BashTool` — `call()` does deny-list substring
  matching (`DENY_LIST` constant: `"rm -rf /"`, `":(){"`, `"mkfs"`,
  `"dd of=/dev"`, `"shutdown"`, `"reboot"`, `"sudo"`), then a `cwd`
  workspace-scope check via `dunce::canonicalize` +
  `starts_with(&self.workspace)`, then dispatches through
  `ctx.proc()?.run(...)`. Also has two free functions worth testing
  directly without needing a full `Ctx`: `render(&ProcOutput) -> Value`
  (caps combined stdout+stderr at 64 KiB with a truncation marker, caps
  stdout/stderr individually too so the cap can't be bypassed) and
  `inside_workspace(workspace: &Path, p: &Path) -> bool` (simple
  `starts_with` check).
- `fs.rs` (197 lines): four tools — `FsReadTool` (offset/limit
  line-slicing + `total_lines` count), `FsWriteTool`, `EditTool`
  (replaces `old_string` with `new_string`, fails with
  `PiiError::Invalid` unless the match count is EXACTLY 1 — both zero
  matches and multiple matches are errors, test both), `MultiEditTool`
  (applies an array of `{old_string, new_string}` edits in order,
  all-or-nothing against one read + one write — if edit N fails, edits
  before it must NOT have been written to disk; verify this
  all-or-nothing property is actually true by reading the code's write
  ordering, not assumed).
- `worktree.rs` (136 lines) — read this file in full; it wasn't
  summarized during grounding for this spec, unlike the other two.
- Testing a `Tool` requires a working `Ctx` with the relevant
  capability granted. The established in-crate pattern (see
  `crates/pii-core/src/fscap.rs`'s `#[cfg(test)] mod tests`, function
  `write_ctx`) is: build a `CapSet` with `caps.kinds =
  CapKind::FS_WRITE` (or `FS_READ`/`PROC` as needed) and a
  `PathScope::new(vec![...])` for the allowed paths, then
  `Ctx::root(caps, Budget::unlimited())`. Reuse this exact pattern
  rather than inventing a new one — check `crates/pii-core/src/ctx.rs`
  and `crates/pii-core/src/cap.rs` for `CapSet`/`CapKind`/`PathScope`'s
  exact API before writing the test helper.
- Test isolation: this crate's existing test convention
  (`fscap.rs`'s `tmp_root()`) uses `tempfile::Builder::new().tempdir_in(...)`
  rooted at a workspace `_tmp` path. Match this pattern's ISOLATION
  approach (a real tempdir per test) but do NOT copy its hardcoded
  `PathBuf::from("_tmp")` literal — that predates
  the F-15 fix requiring `CARGO_MANIFEST_DIR`-relative paths in NEW
  test code (spec-53's own test file, `mem_store_contract.rs`, already
  established `concat!(env!("CARGO_MANIFEST_DIR"), "/../../_tmp")` as
  the correct modern pattern — follow that, not `fscap.rs`'s older one).

## Requirements

### Functional

- R1. `bash.rs`: test `render()` directly (no `Ctx` needed) — combined-
  output truncation at 64 KiB with the marker appended, and the
  untruncated case returning the marker absent. Test `inside_workspace`
  directly. Test `BashTool::call` via a `Ctx` with `PROC` capability:
  deny-list rejection for at least 2 of the 7 listed patterns (returns
  `PiiError::Denied`), a `cwd` outside the workspace rejected
  (`PiiError::Denied`), and a real successful command execution (e.g.
  `echo hello`) returning the expected `exit_code`/`output` shape.
- R2. `fs.rs`: for each of the 4 tools, at least one success-path test
  through a real `Ctx` with the right capability. For `EditTool`
  specifically: one test for the zero-matches-is-an-error case, one for
  the multiple-matches-is-an-error case (both `PiiError::Invalid`), one
  for the exactly-one-match success case. For `MultiEditTool`: a
  multi-step success case, AND a case where a later edit's
  `old_string` doesn't match — assert the file on disk is UNCHANGED
  from before the call (proving the all-or-nothing property is real,
  not just assumed from the doc comment).
- R3. `worktree.rs`: read the file first, then write tests covering its
  actual public API and error paths — this spec deliberately does not
  pre-specify worktree.rs's test cases since grounding didn't cover it;
  match the same rigor as R1/R2 (real `Ctx`-driven success paths plus
  at least one real error path per public function).

### Non-Functional

- N1. Do NOT modify any file under `crates/pii-tools/src/` — test-only
  addition (inline `#[cfg(test)] mod tests` blocks appended to the
  existing files, matching this crate's own convention — appending an
  in-file test module is not the kind of change this guardrail means to
  forbid; changing any NON-test code in these files is).
- N2. Standard test-module clippy allows apply
  (`unwrap_used`/`expect_used`/`panic`/`indexing_slicing`), matching
  every other `#[cfg(test)]` module's convention in this crate.
- N3. Each test that touches the filesystem must use its own isolated
  tempdir (per N-3 in spec-53's precedent) — no shared mutable state,
  no test-order dependency.

## Architecture

No new files — appends `#[cfg(test)] mod tests { ... }` blocks to the
END of `bash.rs`, `fs.rs`, and `worktree.rs`, matching this crate's
existing pattern in its other 4 files.

## TDD Contract

| Test Name (illustrative) | Input | Expected Output |
|---|---|---|
| `render_truncates_large_output` | >64 KiB combined stdout+stderr | truncated at cap, marker appended |
| `render_untruncated_when_small` | small output | no marker, exact content |
| `bash_denies_dangerous_command` | e.g. `"sudo rm -rf /"` | `PiiError::Denied` |
| `bash_denies_cwd_outside_workspace` | `cwd` pointing outside | `PiiError::Denied` |
| `bash_runs_real_command` | `echo hello` | `exit_code: 0`, output contains `hello` |
| `fs_read_respects_offset_limit` | a multi-line file, `offset`/`limit` | correct line slice + `total_lines` |
| `edit_requires_exactly_one_match` | 0 matches, then 2 matches | both `PiiError::Invalid` |
| `edit_succeeds_on_unique_match` | exactly 1 match | file updated on disk |
| `multi_edit_is_all_or_nothing` | edits where a later one fails | file unchanged on disk |
| worktree.rs tests (names TBD by implementer per R3) | per actual API | success + error paths |

## Exit Criteria

- [ ] `cargo test -p pii-tools`
- [ ] `cargo test --workspace --lib --bins --tests`
- [ ] `cargo test --workspace --doc`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo fmt --all --check`
- [ ] `cargo run -q -p xtask -- silent-noop-check`
- [ ] `cargo run -q -p xtask -- lint-files`
- [ ] `grep -q "cfg(test)" crates/pii-tools/src/bash.rs`
- [ ] `grep -q "cfg(test)" crates/pii-tools/src/fs.rs`
- [ ] `grep -q "cfg(test)" crates/pii-tools/src/worktree.rs`

## Guardrails

- G1. Do NOT modify any non-test code in `bash.rs`, `fs.rs`, or
  `worktree.rs` — only append `#[cfg(test)]` modules.
- G2. Do NOT touch any other file in `crates/pii-tools/src/`.
- G3. Do NOT create `crates/pii-tools/tests/` — this crate's convention
  is inline test modules; match it, don't introduce a parallel
  integration-test directory.
- G4. Do NOT touch `specs/` (read-only for the workhorse).
- G5. Do NOT commit — leave the diff in the working tree for the
  orchestrator to review and commit.
- G6. No `unsafe` code (`#![forbid(unsafe_code)]`, workspace-wide).
