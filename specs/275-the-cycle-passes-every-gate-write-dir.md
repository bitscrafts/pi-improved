# Spec 275: the cycle's floor is wired to every volume the gate writes

## Overview
**Project**: /projects/pi-improved
**Phase**: production — queue robustness. This is the follow-up spec 273's own review owed.

Spec 273 moved the gate's disk floor from `df /projects` to
`disk-floor.sh 20000 /projects $GATE_WRITE_DIRS`, so the floor would read the two `llvm-cov-target`
symlinks that land on the host root. Its review then recorded a probe that **survived**: delete the
` $GATE_WRITE_DIRS` argument at `cycle.sh:325` and every exit criterion of 273 still returns 0, and every
case of `disk-floor.test.sh` still prints PASS. Nothing in the tree reads *the arguments the live call
really passes*: `disk-floor.test.sh:65` asserts that `cycle.sh` **calls** `disk-floor.sh`,
`:67` asserts that `env.sh` **defines** `GATE_WRITE_DIRS`, and `disk-floor.sh` judges whatever paths it is
handed. So `GATE_WRITE_DIRS` can be defined, exported, documented — and dead, with no check able to fail.

A cycle wired to `/projects` alone is exactly the 2026-10-24 defect 273 was written to close (a gate that
fills the host root while the floor watches another volume), reachable again by a one-token edit that turns
nothing red.

## Verified Premises
- **CONFIRMED (measured 2026-10-25, clean `git archive HEAD` export of `794641f` under
  `_tmp/spec275-measure/src`)**: with `cycle.sh:325` mutated to
  `DISK=$(bash scripts/autonomy/disk-floor.sh 20000 /projects); DRC=$?`, **9 of 9** shell exit criteria of
  spec 273 return 0 and `bash scripts/autonomy/tests/disk-floor.test.sh` reports `0 FAIL`. The tenth
  criterion (`lint-files`) is a format check and cannot see an argument list either.
- **CONFIRMED** `cycle.sh:325` is
  `  DISK=$(bash scripts/autonomy/disk-floor.sh 20000 /projects $GATE_WRITE_DIRS); DRC=$?`, and
  `cycle.sh:324` is a **comment** that also contains the string `disk-floor.sh` — a checker that greps for
  the first match finds the comment, not the call.
- **CONFIRMED** `env.sh:53` defines and exports `GATE_WRITE_DIRS` with five paths
  (`_tmp/gate-target`, that dir's `llvm-cov-target`, `_tmp/tools-gate`, that dir's `llvm-cov-target`,
  `/tmp`). `env.sh` contains no `mkdir`, `cd`, `rm` or `mv`, so it is safe to source in a subshell.
- **CONFIRMED** `disk-floor.sh` judges exactly the paths it is given and exits 1 on `LOW`/`MISSING`, so the
  argument list **is** the behaviour: a quoted `"$GATE_WRITE_DIRS"` collapses five paths into one bogus
  path and would refuse every gate.
- **CONFIRMED** `cycle.sh:322` is the `GATE_ON=1` decision and the floor call sits below it, inside the
  `if [ -n "$GATE_ON" ]` block: the floor guards the gate, so it must not move above that decision.
- **NOT CLAIMED**: that 20000 MB or the five-path list are the right values (273's business, unchanged
  here); that `cycle.sh` should be exercised end to end (that dispatches a paid worker and is not a unit
  test); that the two `verify-*-target` dirs should join the list (a separate finding, needs the
  operator's decision on what may be reclaimed).

## Requirements
### Functional
- **R1 — a check that can fail, reading the live path.** New
  `scripts/autonomy/tests/cycle-disk-wiring.test.sh`, in the style of `unlisted-changes.test.sh`
  (one `ok <name>` / `FAIL <name>` line per assertion, a final `<N> ok, <M> FAIL` line, exit 0 only when
  `M` = 0, scratch created by the test under `_tmp/` and removed only by its own `trap`). It must
  **extract the invocation from `scripts/autonomy/cycle.sh` at run time** and evaluate the argument text it
  finds — never a copy of the line typed into the test, and never a `grep` for a variable name.
  Cases:
  - **(a) one call site.** Among the **non-comment** lines of `cycle.sh` (first non-blank character is not
    `#`) exactly **one** contains `disk-floor.sh`; the count is asserted, so a second unmeasured call site
    fails. The comment skip is proved by a fixture copy that adds a `# disk-floor.sh` line above the call:
    the extracted argument list must still be the real one.
  - **(b) the arguments, executed.** Take the text after `disk-floor.sh` on that line up to the closing
    `)`, `eval "set -- $ARGS"` in a subshell with `GATE_WRITE_DIRS` set to two planted directories the test
    creates under `_tmp/`, and assert: the first positional argument is a numeric floor; `/projects` is
    among the arguments; **every** planted directory from `GATE_WRITE_DIRS` is among the arguments as its
    own argument. This is the assertion probe d5 breaks.
  - **(c) the live list, sourced.** Source `scripts/autonomy/env.sh` in a subshell and assert
    `GATE_WRITE_DIRS` yields at least five path arguments after the floor and `/projects` — so (b) reads the
    environment, not a literal.
  - **(d) the floor guards the gate.** The invocation's line number is greater than the line number of the
    `GATE_ON=1` decision in `cycle.sh`.
  - **(e) CONTROL — the checker reddens a planted unwiring.** Run (b)'s assertions against a scratch copy
    of the extracted text with ` $GATE_WRITE_DIRS` removed and assert they **FAIL**; print
    `ok the checker reddens a planted unwiring`.
  - **(f) CONTROL — the near-miss stays green.** A correctly wired variant carrying an extra path
    (`... 20000 /projects $GATE_WRITE_DIRS "$EXTRA"`, with `EXTRA` set to an existing directory) must PASS
    the same assertions; print `ok the near-miss stays green`. A checker stricter than the property is a
    broken check.
  - **(g) CONTROL — a quoted expansion is refused.** The same assertions against `... /projects
    "$GATE_WRITE_DIRS"` must FAIL, because one quoted argument collapses five paths into one and
    `disk-floor.sh` would report it `MISSING`.
- **R2 — one source of truth.** The test derives its expectation from the live line and from `env.sh`; it
  must not restate the five paths. If the call site later moves or gains an argument, the test follows it
  and only the property (every write dir reaches the helper as its own argument) is asserted.
- **R3 — no behaviour change.** `cycle.sh`, `disk-floor.sh` and `env.sh` are **not** modified by this spec:
  the property already holds at HEAD. If the check reddens on the unmodified tree, that is a genuine shape
  mismatch — the worker STOPS and reports it, naming the line, rather than editing the call, the floor or
  the variable list to make the test pass.

### Non-Functional
- Shell only, no Python, no `cargo` invocation from the test; scratch strictly under `_tmp/`; every file
  under the 450-line effective gate; the test under 200 lines.
- The test deletes nothing but the scratch directory it created in this run (its own `trap`), like
  `unlisted-changes.test.sh`. No `rm -rf`, no `mv` of a tracked file.

## TDD Contract
1. **RED (planted, in-test).** Cases (e) and (g) are the red controls: the test must print both named `ok`
   lines, which means the checker itself was made to fail on a defective call and to pass on a correct,
   differently-shaped one. A test whose controls are absent is not evidence.
2. **RED (the real mutation, owed by the orchestrator at review).** On a clean tree, delete ` $GATE_WRITE_DIRS`
   from `cycle.sh:325` and run the test: it must print at least one `FAIL` line and exit non-zero.
   Restore the line and prove it byte-identical (`sha256sum` before and after); re-run to green.
   Record both readings in `## Closure`. A red whose failing assertion is not the argument assertion is
   not evidence for this spec.
3. **GREEN.** On the unmodified tree the test prints `N ok, 0 FAIL`, and the neighbours
   (`disk-floor.test.sh`, `unlisted-changes.test.sh`) still print `0 FAIL`.

## Exit Criteria
- [ ] `bash scripts/autonomy/tests/cycle-disk-wiring.test.sh 2>&1 | tail -3 | grep -q ", 0 FAIL"`
- [ ] `! bash scripts/autonomy/tests/cycle-disk-wiring.test.sh 2>&1 | grep -q "^FAIL"`
- [ ] `bash scripts/autonomy/tests/cycle-disk-wiring.test.sh 2>&1 | grep -q "ok the checker reddens a planted unwiring"`
- [ ] `bash scripts/autonomy/tests/cycle-disk-wiring.test.sh 2>&1 | grep -q "ok the near-miss stays green"`
- [ ] `bash -n scripts/autonomy/tests/cycle-disk-wiring.test.sh && bash -n scripts/autonomy/cycle.sh`
- [ ] `git diff --quiet -- scripts/autonomy/cycle.sh scripts/autonomy/disk-floor.sh scripts/autonomy/env.sh`
- [ ] `bash scripts/autonomy/tests/disk-floor.test.sh 2>&1 | tail -1 | grep -q "0 FAIL"`
- [ ] `bash scripts/autonomy/tests/unlisted-changes.test.sh 2>&1 | tail -1 | grep -q "0 FAIL"`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

## Guardrails
- Do **not** modify `cycle.sh`, `disk-floor.sh` or `env.sh`. The property holds at HEAD; a red on the
  unmodified tree means STOP and report the mismatch (R3).
- Do **not** weaken the argument assertion into a `grep` for `GATE_WRITE_DIRS` in `cycle.sh`. That text
  check would redden on d5 while certifying a call that hands the helper a quoted, single, bogus path —
  case (g) exists precisely to refuse that shape.
- Do not add the two `verify-*-target` directories to `GATE_WRITE_DIRS`; that is a separate finding with an
  operator decision attached.
- Delete nothing (scratch removal via the test's own `trap` only), move nothing, commit nothing — the
  cycle commits after the gate.

## Closure
Filled by the orchestrator at review: the mutation d5's red output with the failing assertion named, the
`sha256sum` before/after of `cycle.sh`, the green re-run, the full gate result, and — for the record — the
reading of `disk-floor.test.sh` after the change.
