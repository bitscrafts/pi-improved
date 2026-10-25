# Spec 266: a queue item's gate measures coverage — the skip switch is pinned off on the gate command

## Overview
**Project**: /projects/pi-improved
**Phase**: production — the queue's own gate (spec 262 follow-up, found 2026-10-24 reading the A128 cycle log: `==> coverage-floor` then `SKIPPED: coverage-floor (QG_SKIP_COVERAGE=1 — the floor was NOT measured in this run`)

Spec 262 made the queue gate every commit with the full step list. `scripts/autonomy/env.sh:40` still exports `QG_SKIP_COVERAGE=1` for every sourcing tool, and `cycle.sh` sources `env.sh` (line 31), so inside the
queue's gate the `coverage-floor` step prints SKIPPED and passes. The floor (`coverage-floor.txt`, 74 percent) is therefore never enforced on a queue commit; it is enforced only when someone runs `bash quality-gate.sh` by hand.

`env.sh` justifies the skip with a measurement from 2026-10-17: "960 seconds for one step", "the full gate costs ~105 MINUTES per item". Today's runs of the same step list on the same host measured differently
(2026-10-24, `/tmp/qgate-259.log`, `/tmp/qgate-260.log`, `/tmp/gate-verdict-263.out`): the whole 15-step gate in about 10 to 12 minutes warm, the coverage step reaching its test-running phase at about 240 to 300 s
(`HEARTBEAT step=coverage-floor elapsed=300s`), coverage reported at 82.71, 82.76 and 82.80 percent. The queue's own bound is `GATE_TIMEOUT` = 3600 s. The premise for skipping no longer holds on a warm cache.

## Verified Premises
- **CONFIRMED** (read) `scripts/autonomy/env.sh:40-41`: `: "${QG_SKIP_COVERAGE:=1}"` then `export QG_SKIP_COVERAGE`; its comment says coverage belongs at the release boundary.
- **CONFIRMED** (read) `gate-steps.txt:39`: the `coverage-floor` step starts with `if [ "${QG_SKIP_COVERAGE:-0}" = "1" ]; then echo "SKIPPED: coverage-floor (QG_SKIP_COVERAGE=1 ..."` and exits 0, so a skipped step reads as a pass in the `PASS: coverage-floor` roll-up.
- **CONFIRMED** (read) `scripts/autonomy/cycle.sh:343` (after spec 262) runs the gate as `( export PATH=...; GATE_STEPS=gate-steps.txt CARGO_TARGET_DIR=... FRESH_GATE_MIN_FREE_MB=12000 timeout "$GATE_TIMEOUT" bash quality-gate.sh --routine )`; it pins `GATE_STEPS` and does not pin `QG_SKIP_COVERAGE`.
- **CONFIRMED** (run, this session) the A128 queue gate: `==> coverage-floor`, `SKIPPED: coverage-floor (QG_SKIP_COVERAGE=1 — the floor was NOT measured in this run,`, while the same tree under `bash quality-gate.sh` measured 82.80 percent.
- **CONFIRMED** (run) `scripts/autonomy/gate-env.sh` (spec 262) defines `gate_steps_file` and `gate_step_labels`; `scripts/autonomy/tests/gate-steps.test.sh` has 8 assertions and passes.
- **NOT CLAIMED** the cold-cache cost: the coverage build was 960 s on 2026-10-17 and a cold run may approach it. It still fits inside `GATE_TIMEOUT` = 3600 s, and a gate that outlives it is `gate-timeout`, never a pass (spec 262 R3).

## Requirements
### Functional
- **R1. Pinned off on the gate command.** `cycle.sh`'s gate command carries `QG_SKIP_COVERAGE=0` beside `GATE_STEPS=gate-steps.txt`, so no environment value can skip the step. `env.sh` is NOT changed (other callers keep the skip by default).
- **R2. The helper decides.** `gate-env.sh` gains `gate_skip_coverage` that prints `0` always (a queue item's commit is gated with coverage measured). `cycle.sh` uses it to build the command (`QG_SKIP_COVERAGE=$(gate_skip_coverage)`), so the rule is in one testable place like the step list.
- **R3. The log says what was measured.** After the gate, `cycle.sh` logs one line `gate coverage: measured` when the log contains a line matching `^coverage-floor: total [0-9.]+% >= floor`, otherwise `gate coverage: NOT MEASURED`. If the gate passed (rc 0) but coverage was NOT measured, the item is NOT committed: it ends `status=needs-review last=gate-coverage-not-measured` with `OUTCOME=gate-coverage-not-measured` and exit code 16. (A green gate that never measured coverage is the failure this spec exists to stop.)
- **R4. A test with controls.** New `scripts/autonomy/tests/gate-coverage.test.sh` (a new file; do NOT edit `gate-steps.test.sh`) asserts each as an `ok`/`FAIL` line: (a) `gate_skip_coverage` prints `0` with `QG_SKIP_COVERAGE=1` exported (the case today); (b) `cycle.sh` contains no gate command that has `GATE_STEPS=gate-steps.txt` without `QG_SKIP_COVERAGE=0` on the same line; (c) CONTROL for (b): a planted copy of the line without the variable makes the same predicate fail; (d) the step text in `gate-steps.txt` skips only when `QG_SKIP_COVERAGE` is `1`, checked by running the step's `if` test in a subshell with the variable set to `0` and to `1` and asserting `SKIPPED` appears only for `1`; (e) `coverage_measured` (a function in `gate-env.sh` that reads a log path and prints `yes` or `no`) says `yes` for a fixture log containing `coverage-floor: total 82.80% >= floor 74.00%` and `no` for a fixture log containing `SKIPPED: coverage-floor`; (f) CONTROL for (e): a log with both lines is `yes` and an empty log is `no`. It prints `N ok, 0 FAIL` and exits 0 only when every assertion passed.

### Non-Functional
- Shell only (bash, `set -u`); no Python added; `bash -n` passes for every edited or new script. Do not edit `gate-steps.txt`, `gate-steps-quick.txt`, `quality-gate.sh`, `env.sh` or `scripts/autonomy/tests/gate-steps.test.sh`.
- Do not run `cycle.sh`, the queue or a gate from the test: it reads files and sources the helper only.

## TDD Contract
| Assertion (an `ok` line of `tests/gate-coverage.test.sh`) | Expected |
|---|---|
| `gate_skip_coverage ignores QG_SKIP_COVERAGE from the environment` | prints `0` with `QG_SKIP_COVERAGE=1` exported |
| `cycle.sh pins coverage on` | no gate command pins `GATE_STEPS=gate-steps.txt` without `QG_SKIP_COVERAGE=0` |
| `CONTROL: the predicate fails on a planted line` | the same predicate flags a temp copy without the variable |
| `the step skips only for 1` | `SKIPPED` appears for `QG_SKIP_COVERAGE=1` and not for `0` |
| `coverage_measured reads the roll-up line` | `yes` for the measured fixture, `no` for the SKIPPED fixture |
| `CONTROL: both lines is yes, empty is no` | as stated |

**The requirement a plausible implementation satisfies only in appearance:** editing `env.sh` to default `QG_SKIP_COVERAGE=0` (every other caller then pays the 5 to 16 minute step), or exporting the variable earlier in `cycle.sh` (a later `source env.sh` or `bash -lc` resets it). The pin must sit on the gate command itself, and assertion (b) requires it on that line.

## Exit Criteria
- [ ] `test -f scripts/autonomy/tests/gate-coverage.test.sh`
- [ ] `bash scripts/autonomy/tests/gate-coverage.test.sh | tail -1 | grep -q " 0 FAIL"`
- [ ] `bash scripts/autonomy/tests/gate-coverage.test.sh > /dev/null`
- [ ] `bash scripts/autonomy/tests/gate-steps.test.sh | tail -1 | grep -q " 0 FAIL"`
- [ ] `bash -n scripts/autonomy/cycle.sh && bash -n scripts/autonomy/gate-env.sh`
- [ ] `grep -q "QG_SKIP_COVERAGE=0" scripts/autonomy/cycle.sh`
- [ ] `grep -q "gate-coverage-not-measured" scripts/autonomy/cycle.sh`
- [ ] `git diff --quiet -- gate-steps.txt gate-steps-quick.txt quality-gate.sh scripts/autonomy/env.sh scripts/autonomy/tests/gate-steps.test.sh`

## Guardrails
- Do NOT change `gate-steps.txt`, `gate-steps-quick.txt`, `quality-gate.sh`, `scripts/autonomy/env.sh` or `scripts/autonomy/tests/gate-steps.test.sh`.
- Do NOT run the queue, `cycle.sh` or a gate from the test. Do NOT commit.

## Follow-ups (not this spec)
`head-builds.sh` checks that HEAD builds, not that it passes the lints; a periodic `--fresh` release-boundary gate; the gate as a `qgate` sub-graph of the generic spec graph.
