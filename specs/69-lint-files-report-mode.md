# lint-files --report Mode for Size Auditing

- **Project**: `<repo>`
- **Crate**: `xtask`
- **Priority**: LOW — tooling convenience, not a bug fix or gate gap.
  Sourced from fable review candidate 11 (both passes, "make the gate
  the single ruler for size reporting"),
  `_docs/06-research/032-fable-pii-review-2026-09-18.md`. Directly
  motivated by a real mistake in this session: the ground-truth doc
  prepared for this review (`_docs/06-research/
  031-fable-review-ground-truth-2026-09-18.md`) listed "13 files over
  450 lines" measured with raw `wc -l`, which does not match the
  actual enforced policy (`xtask lint-files`'s effective-line count,
  which excludes comments/blanks/test bodies) — none of those 13
  files actually violate the real gate. This spec prevents that
  mistake from recurring.

## Overview

`xtask lint-files` (see the companion spec-67 for its core counting
logic and a real bug fix in it) currently only checks pass/fail
against the 450/380 thresholds and reports failures — it has no mode
to simply list the largest files by effective line count for informal
auditing or ground-truth-doc preparation. Anyone wanting that today
falls back to raw `wc -l`, which is not the enforced metric and can
produce misleading lists (as this session's own prep doc did).

Ground truth (verify yourself, re-read the file — this spec assumes
spec-67 has already landed its `count_effective_lines` bug fix; if it
has not, coordinate or note the dependency in your implementation
report):
- `xtask/src/lint_files.rs` — the existing pass/fail check logic, to
  extend with a new reporting mode rather than duplicate the counting
  logic.
- `xtask/src/main.rs` — how `lint-files` is currently wired as an
  xtask subcommand, to add the new flag consistently with this
  project's existing subcommand/flag conventions.

## Requirements

### Functional

- R1. Add a `--report [N]` flag to the `lint-files` subcommand
  (default `N` = 20 if omitted, or whatever this project's convention
  for optional numeric flags with defaults already is — check an
  existing xtask subcommand with a similar flag shape before
  inventing a new pattern). When present, instead of (or in addition
  to — your choice, document which) the normal pass/fail check, print
  the top-N files by effective line count, each line showing both the
  effective count and the raw `wc -l` count side by side, e.g.:
  ```
  360  627  crates/pii-store/src/lib.rs
  341  801  crates/pii-model/src/ollama.rs
  ...
  ```
  (effective, then raw, then path — or whatever column order reads
  most naturally; be consistent and document the format in a doc
  comment).
- R2. `--report` mode exits 0 regardless of whether any file is over
  the threshold — it's a reporting tool, not a gate check. The normal
  (no-flag) invocation's pass/fail behavior must be completely
  unchanged.
- R3. Update `_docs/06-research/031-fable-review-ground-truth-2026-09-18.md`'s
  file-size list to use `cargo run -q -p xtask -- lint-files --report`
  output instead of the raw `wc -l` measurements it currently has —
  this is a docs-only correction, do not touch any source code beyond
  `xtask` itself for this. State clearly in the doc (or in a note
  alongside it) that the list was corrected and why.

### Non-Functional

- N1. Do NOT change the counting logic itself in this spec (that's
  spec-67's job, if not already landed) — this spec only adds a new
  OUTPUT mode over the existing (or spec-67-fixed) counter.
- N2. `--report`'s output must be stable/deterministic for the same
  input (same file set, same sizes) — sort consistently (e.g.
  effective-count descending, then path alphabetically as a tiebreak).

## Architecture

Described inline in Overview above (file paths and function names given there constitute the architecture context for this change).

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| `report_mode_lists_top_n_by_effective_lines` | a fixture set of files with known effective-line counts, `--report 3` | exactly 3 lines, sorted descending by effective count |
| `report_mode_exits_zero_even_with_oversized_files` | a fixture file exceeding 450 effective lines, `--report` | exit code 0 (not the normal gate's failure exit) |
| `report_mode_default_n_is_reasonable` | `--report` with no explicit N | uses the documented default (20 or whatever was chosen) |
| `normal_mode_unaffected_by_report_flag_addition` | existing lint-files invocation with no `--report` flag | behavior identical to before this spec (still a pass/fail gate check) |

## Exit Criteria

- [ ] `cargo run -q -p xtask -- lint-files --report | head -5` (produces readable output, does not error)
- [ ] `cargo run -q -p xtask -- lint-files` (normal mode still exits correctly based on pass/fail, unaffected by the new flag's addition)
- [ ] `cargo test -p xtask`
- [ ] `cargo build -p xtask`
- [ ] `cargo clippy -p xtask -- -D warnings`
- [ ] `cargo fmt --check`

## Guardrails

- Do NOT change `count_effective_lines`'s logic in this spec (defer to
  spec-67, or coordinate if implementing both together).
- Do NOT modify any file under `specs/`.
- The doc correction (R3) touches ONLY the 031 ground-truth doc's
  file-size list — do not edit any other part of that document, and
  do not touch the 032 findings doc.
- No emojis, no AI-tool mentions in any comment or commit message.
