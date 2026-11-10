# Spec: every crate is measured against one standard, and the measurement is in the gate

## Overview

The operator's directive: *"thats a work on optimization and refactor! things we need to do with every
pii's crate!"*

Measured today, 2026-11-04, by walking `crates/*/`:

```
crates            22
source LOC        146,663
tests (#[test])   1,849
test files        315
files over 450    57        ← the rule exists; it is only enforced on files a row touches
```

The rule is enforced *per changed file* (`lint-files`), so a file that was already over the line stays
over it forever, and every row touching it must fight a ratchet it never started. The largest
violators are also the largest crates:

```
pii-server    51,471 LOC   101 src files   19 over 450   tests/coder_container_bounded.rs 483 …
pii-cli       24,217        59              9            a test file at 939 lines
pii-graph     17,298        41              5            validate.rs 524 · render.rs 516
pii-store      6,774        20              4            lib.rs 829 (!) · queue.rs 527
pii-observe    3,554        15              2            trajectory.rs 726 · names.rs 648
pii-core       3,123        13              3            observe.rs 711 · childproc.rs 490
pii-worker     2,083         8              1            control.rs 704
pii-model      3,743        10              2            tests/provider_kinds.rs 671
pii-learn      6,884        18              4            tests/learn.rs 564 · tests/promote.rs 553
pii-memory     5,498        22              3            tests/entities.rs 482 · tests/tiering.rs 482
pii-wasm       1,189         5              2            host.rs 480 · tests/host.rs 487
pii-tools      3,293        10              1            mqtt.rs 550
pii-net        4,156        17              1            tests/egress.rs 478
pii-web          599         1              1            main.rs 599
pii-* (8 more)   …           …              0
```

57 files is not a row. It is a program, and the program needs an instrument that makes it continuous
rather than a once-a-year crusade.

**Project**: <repo>
**Phase**: workspace-wide refactor

## Requirements

- **R1 — one command measures the whole workspace.** `cargo run -q -p xtask -- crate-audit` prints,
  per crate: source files, source LOC, largest file, test files, `#[test]` count, and the files over the
  450-line budget. It is a measurement, so it runs in seconds and touches nothing.
- **R2 — a ratchet, not a purge.** The audit records the current count of over-budget files per crate
  (a checked-in baseline) and **exits non-zero if any crate exceeds its baseline**. Debt may not grow
  while it is being paid: new work keeps the rule it already had, and a crate's count may only go down
  until it reaches zero.
- **R3 — the standard is written once.** `docs/03-reference/crate-standard.md` states, per point, what
  is measured and what the check is: file size · tests exist and can fail · no duplicated module logic ·
  minimal public surface · observability through the seam · no dead module · no `unwrap` on a runtime
  path. The doc names the check for each point, or says "not measured yet" — a standard with unmeasured
  clauses is a wish list and must say so out loud.
- **R4 — the audit runs in the gate.** `fresh-gate` gains one step: `crate-audit --ratchet`. A crate
  that grows its debt reddens the gate, with the crate and the file named.
- **R5 — the debt is ordered, not forgotten.** The 57 over-budget files become rows, worst crate first,
  each row scoped to one crate and its own verification. The audit's output is the row generator: a
  row must name the files it clears, and clearing them must be the criterion.
- **R6 — no behaviour change is smuggled in.** A refactor row states, per file it splits, what it does
  *not* change, and its proof is the existing test suite plus the crate's own tests unchanged in count
  or increased. A split that deletes tests is a regression wearing a refactor's clothes.

## TDD contract

| Test | Input | Expected |
|---|---|---|
| `the_audit_measures_every_crate` | 22 crates present | one row per crate, and the totals match a recount |
| `the_audit_refuses_growth` | a crate with a planted 451-line file, baseline 0 | rc != 0, naming the file |
| `the_audit_allows_payment` | baseline 3, actual 2 | rc == 0 |
| `CONTROL_a_new_file_under_the_budget_is_not_a_violation` | a 400-line file | rc == 0 |
| `control_the_baseline_cannot_be_raised_by_the_audit_itself` | `--update-baseline` without an explicit flag | refused |

## Exit Criteria

- [ ] `cargo run -q -p xtask -- crate-audit >/dev/null`
- [ ] `test -f _docs/05-operations/crate-audit-baseline.tsv`
- [ ] `test -f docs/03-reference/crate-standard.md`
- [ ] `cargo test -q -p xtask --test crate_audit 2>&1 | grep -qE 'test result: ok. [1-9][0-9]* passed'`
- [ ] `grep -q 'crate-audit' xtask/src/fresh_gate.rs`

## Guardrails

- Do NOT delete tests to clear a file over the budget. The count is part of the measurement, and the
  audit prints it precisely so a suspicious drop is visible.
- Do NOT raise a budget per file. pii-store's `lib.rs` at 829 lines is not a file that needed more
  room; it is a file that holds too much.
- Do NOT let the audit write its own baseline without `--update-baseline`, and never in the gate.
- Do NOT treat the audit as the refactor. It makes debt visible and stops its growth; the rows pay it.
- Do NOT split a file into `foo.rs` + `foo_part2.rs`. A split earns its name: a module boundary, a
  test module, or a type that was doing two jobs.

## Files

- `xtask/src/main.rs`
- `xtask/src/crate_audit.rs`
- `xtask/tests/crate_audit.rs`
- `xtask/src/fresh_gate.rs`
- `_docs/05-operations/crate-audit-baseline.tsv`
- `docs/03-reference/crate-standard.md`
