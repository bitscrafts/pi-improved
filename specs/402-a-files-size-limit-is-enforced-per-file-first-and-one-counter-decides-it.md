# Spec: a file's size limit is enforced per file, first, and one counter decides it

## Overview
Row 401's worker finished, its tests passed, and the gate failed 25 minutes later on one thing: `discord.rs` was 465 lines against a 450 limit.
The user's verdict: file lines must be enforced, and folder modules are the way to go. The class is measured, not a one-off:
- Of 17 failed gates in 1,387 cycle logs, 6 were `crate-audit`, and `crate-audit` runs after about 25 minutes of build, test and coverage.
- Two counters disagree. `lint-files` counts effective lines, skips `tests/` and `#[cfg(test)]`, and PASSED on the 465-line file.
  `crate-audit` counts physical lines in `src/` and `tests/` and caught it. The worker has no way to know which one will judge it.
- The ratchet compares a per-CRATE count (`crate-audit-baseline.tsv`: `crate<TAB>over_budget`). A file that is already over budget may therefore
  grow without limit, and one over-budget file may be swapped for another, because the count stays equal.
- 60 files are over the limit today (the audit's own total), 19 of them in `pii-server`.
- Files sit at the limit: `lib.rs`, `validate.rs` and `mqtt.rs` are at exactly 450 effective lines. There is no headroom, so the next edit fails.

**Project**: <repo>
**Phase**: gate discipline (follows 387, which created the crate-level ratchet)
**Depends on**: 387

## Requirements
- R1 One counter. `lint-files` and `crate-audit` use the same function for the number they judge. The rule is PHYSICAL lines (`wc -l`) in `src/` and
  `tests/`, limit 450. Physical is what a worker sees with `wc -l`, and what the stricter audit already used. The effective count stays as a report column only.
- R2 Per-file ratchet. The baseline records each over-budget file by PATH and its line count (`path<TAB>lines`). The check fails when:
  a file not in the baseline is over 450; a baselined file has more lines than recorded; a baselined file is at or under 450 and still listed;
  a baselined path no longer exists and is still listed. Each failure names the file, its count and the recorded count.
- R3 `--update-baseline` writes the file on first creation and afterwards only LOWERS or removes entries. It refuses to add a path or raise a count.
- R4 Fail first. `crate-audit --ratchet` and `lint-files` are the first steps of `gate-steps.txt`, before `fmt`, so a size failure costs seconds.
- R5 Headroom report. `lint-files --report` lists files within 30 lines of the limit, so a spec author can see where new code must not go.
- R6 `_docs/05-operations/crate-audit-baseline.tsv` is regenerated from the CURRENT tree in the new format with the one-time `--migrate-from-crate-counts` mode
  (it may be run once, refuses when the new format already exists, and writes nothing if any file is over its recorded crate's count).

## Architecture
```
one fn physical_lines(path) -> usize        used by lint-files AND crate-audit (no second implementation)
baseline: path<TAB>lines                    may only shrink
gate-steps.txt: crate-audit --ratchet, lint-files, THEN fmt, clippy, build, test ...
```

## TDD Contract
| Test Name | Input | Expected Output |
|---|---|---|
| `a_new_file_over_450_is_refused_naming_it` | a 451-line file not in the baseline | `Err` naming the path and 451 |
| `CONTROL_a_450_line_file_passes` | a 450-line file not in the baseline | `Ok` |
| `a_baselined_file_that_grew_is_refused` | recorded 500, now 501 | `Err` naming the path, 501 and 500 |
| `a_baselined_file_is_not_replaced_by_another_over_budget_file` | baseline lists A (500); A shrinks to 440 and B grows to 460 | `Err` for B, and A's stale entry |
| `a_baselined_file_under_the_limit_must_leave_the_baseline` | recorded 500, now 440, still listed | `Err` naming the stale entry |
| `a_baselined_path_that_is_gone_must_leave_the_baseline` | listed path does not exist | `Err` naming it |
| `update_baseline_refuses_to_add_or_raise` | a new over-budget path / a higher count | `Err`, file unchanged |
| `both_commands_use_the_same_counter` | a file whose comments make effective <= 450 and physical = 465 | both refuse it |
| `size_checks_run_before_the_build` | parse `gate-steps.txt` | `crate-audit` and `lint-files` precede `fmt` and `clippy` |
| `the_baseline_migration_refuses_when_a_file_exceeds_its_crate_count` | tree with one file over its old crate's recorded count | `Err`, nothing written |

Acceptance test: `a_baselined_file_is_not_replaced_by_another_over_budget_file`. It is the case the old per-crate count passes (same count) and
the one that matters: a plausible implementation that keeps comparing counts satisfies every other row and fails only here.
Every refusal needs its near-miss (`CONTROL_`) in the same file.

## Exit Criteria
### New in this row
Each is verified to FAIL before the change. A criterion that already passes is a Guardrail, never a criterion.
- [ ] `cargo test -q -p xtask --test file_size_ratchet 2>&1 | grep -qE 'test result: ok. [1-9][0-9]* passed'`
- [ ] `test -f xtask/src/file_size.rs`
- [ ] `head -n 12 gate-steps.txt | grep -n 'crate-audit' | grep -q .`
- [ ] `head -n 1 _docs/05-operations/crate-audit-baseline.tsv | grep -qP '^path\tlines$'`

## Guardrails
- Keep green (they pass today, so they are not criteria): `cargo test -q -p xtask --lib` and `cargo run -q -p xtask -- lint-graph-kinds`.
- Do NOT raise a limit or add a baseline entry to make the work pass. The migration may only record what exists.
- Do NOT put the new code in `crate_audit.rs` (262 lines) or `lint_files.rs` (336 lines) if it would take either past 400: it goes in `xtask/src/file_size.rs`.
- Do NOT create a second line counter. Do NOT skip `tests/`.
- Every dirty path must be listed under Files: list a file before you touch it.

## Files
- `xtask/src/file_size.rs`
- `xtask/src/crate_audit.rs`
- `xtask/src/lint_files.rs`
- `xtask/src/main.rs`
- `xtask/tests/file_size_ratchet.rs`
- `gate-steps.txt`
- `_docs/05-operations/crate-audit-baseline.tsv`
- `docs/03-reference/crate-standard.md`
- `xtask/src/lib.rs`
- `xtask/tests/crate_audit.rs`
