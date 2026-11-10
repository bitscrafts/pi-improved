# Spec: nothing writes scratch outside the tree, and the lint that says so is real

## Overview

`lint-scratch` is the step that is supposed to keep test scratch inside the repository's `_tmp/`, where the
project's rules put it and where `cargo clean` and an operator can see it. Measured 2026-10-11
(`_docs/06-research/059-opus-vision-judgement-2026-10-11.md` §0.1, F6): the check only verifies that a
`tempdir_in(` call aimed at `_tmp` creates its directory first (`xtask/src/lint_scratch.rs:1-8`). It does
**not** forbid `std::env::temp_dir()`, and `crates/pii-web/src/main.rs:505` writes there — **457
`/tmp/pii-web-mem-*` directories** had accumulated at the time of measurement. A lint that passes while the
live path does the forbidden thing is worse than no lint (HARD RULE 0b).

**Project**: <repo>
**Phase**: post-vision hardening (finding 059 F6)
**Source of the finding**: `_docs/06-research/059-opus-vision-judgement-2026-10-11.md` §0.1, §2.2 (F6), §2.3

## Requirements

### Functional
- **R1** No code under `crates/` or `xtask/` calls `std::env::temp_dir()` (or an equivalent outside-the-tree
  scratch root) for test or runtime scratch. The one confirmed site (`crates/pii-web/src/main.rs:505`) is
  moved under the tree's scratch root, using the same resolution the rest of the project uses (a runtime
  directory derived from the workspace, not a hardcoded `/tmp`). If the site needs a directory that survives
  a process, say so in the report and place it under the runtime dir, not `/tmp`.
- **R2** `xtask`'s scratch lint becomes an **enforcement**: it fails on a `temp_dir()` call in the tree, with
  the file and line in the message. It must print the count it checked (the existing step does) and it must
  be RED on a probe before it is trusted.
- **R3** The lint keeps its current job too (a `tempdir_in` aimed at `_tmp` must create it): one step, both
  rules, both with a control.
- **R4** The 457 accumulated directories are **not** deleted by the implementer. Report the count and the
  path; the operator decides (`rm -rf /tmp/...` is banned by HARD RULE 0).
- **R5** The check's own control is committed: a test (in `xtask/tests/`) that feeds the lint a source string
  containing `env::temp_dir()` and asserts it is refused, plus a near-miss that must pass (a `tempdir_in`
  aimed at `_tmp`) — a rule that suppresses something needs a case that still takes the normal path.

### Non-Functional
- No new dependency. The check must stay fast (it reads sources, it does not build).
- A false positive is a defect: legitimate uses (e.g. a comment, a doc string, a path built from
  `PII_TMP`) must be excluded explicitly, and the exclusion must be visible in the check, not by a
  file-wide allowlist entry that hides the class again.

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| `temp_dir_is_refused` (new, `xtask`) | a source line with `std::env::temp_dir()` | the check fails, naming file and line |
| `tempdir_in_under_the_tree_still_passes` (new, control) | a `tempdir_in(<workspace>/_tmp/…)` site | the check passes |
| the same check on the pre-fix tree (scratch copy) | the real `crates/pii-web/src/main.rs` site | RED — the check names that file |

## Exit Criteria

- [ ] `cargo run -q -p xtask -- lint-scratch 2>&1 | tail -2 | grep -q 'files checked'`
- [ ] `! grep -rn 'temp_dir()' crates --include=*.rs | grep -q .`
- [ ] `cargo test -p xtask 2>&1 | tail -2 | grep -q 'test result: ok'`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -1 | grep -qv 'error'`

## Guardrails

- Do NOT delete anything in `/tmp` and do NOT add a cleanup that removes another process's directories:
  report the count instead (R4).
- Do NOT silence the new rule with a file-wide allowlist entry for the offending file — fix the call site.
- Do NOT widen the lint into guesswork about every path literal; it is about the scratch **root**.
- Do NOT create, edit or rewrite anything under `specs/`; do NOT commit; scratch under `<scratch>/`.

## Closure

(outcome, the count of `/tmp` directories found, the probe that reddens the new rule, and the near-miss
control — filled in when the item is closed.)
