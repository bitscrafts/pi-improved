# Spec 262: a queue item's commit is gated by the full gate, not by fmt, clippy and build alone

## Overview
**Project**: /projects/pi-improved
**Phase**: production — the queue's own gate (found 2026-10-24 while reviewing spec 259's commit)

The queue gates each item with `quality-gate.sh --routine`. `scripts/autonomy/env.sh:47` defaults `GATE_STEPS` to `gate-steps-quick.txt`, which runs three steps: `fmt`, `clippy`, `build`. The full list
(`gate-steps.txt`) has fifteen: `fmt clippy "clippy (hal-gpiod)" build "test (lib/bins/tests)" "test (doc)" coverage-floor lint-files lint-scratch lint-observability reachability-check deps-check net-check doc-drift-check devbox-check`.
`quality-gate.sh` itself documents that "the commit and release boundaries use the full gate-steps.txt", and `cycle.sh`'s log line still says "fresh-gate PASS", so a commit reads as fully gated when it was not.

Measured consequence, twice in one day: the cycle logged `quality-gate: ALL PASS` (three steps) and committed
- spec 257 as `f52fb7f` while `lint-observability` was red (`crates/pii-cli/src/cmd_sys.rs` had 10 raw prints against a budget of 8);
- spec 259's worker left an older tracked-graph test failing (`V0: unknown kind 'judge'`), which only the item's own criterion run caught.
The full gate on those trees found the first at once. The orchestrator repaired both by hand; the queue should not depend on that.

## Verified Premises
- **CONFIRMED** (read) `scripts/autonomy/env.sh:46-48`: `: "${GATE_STEPS:=gate-steps-quick.txt}"` then `export GATE_STEPS`; the comment says "Set GATE_STEPS=gate-steps.txt deliberately when a full run is intended."
- **CONFIRMED** (read) `scripts/autonomy/cycle.sh:339`: `( export PATH=...; CARGO_TARGET_DIR=/projects/pi-improved/_tmp/gate-target FRESH_GATE_MIN_FREE_MB=12000 timeout "$GATE_TIMEOUT" bash quality-gate.sh --routine ) >>"$LOG" 2>&1`; `GATE_TIMEOUT` defaults to 3600; a rc of 124 is `gate-timeout` (not a failure, attempts not incremented), a nonzero rc is `gate-failed`.
- **CONFIRMED** (run, 2026-10-24) the FULL routine gate takes about 10 to 12 minutes on the warm cache (spec 259: 14 steps; spec 260: 15 steps, coverage 82.71 percent), against the "~45-48 min" the cycle comment cites for the FRESH export. So the full routine list fits inside `GATE_TIMEOUT=3600` with room.
- **CONFIRMED** (read) `quality-gate.sh` selects the list from `$GATE_STEPS` and prints `quality-gate: ALL PASS` only when every listed step passed; an empty list is a failure.
- **CONFIRMED** (read) `scripts/autonomy/selftest.sh` exists and mentions `gate` seven times; `scripts/autonomy/tests/classify-failure.test.sh` (spec 254) is the idiom for a shell test with controls.
- **MEASURED** (2026-10-24, `grep -rn 'GATE_STEPS\|gate-steps-quick' scripts/ *.sh`): the only consumer of the quick list is `scripts/autonomy/env.sh:47` (the default) feeding `cycle.sh:339`, which is the ONLY `quality-gate.sh --routine` command in `cycle.sh` (count 1); `quality-gate.sh` defaults to the full list itself. `cycle.sh:342,351,357` say `fresh-gate PASS|TIMED OUT|FAILED`, which is wrong for a routine run (R2 renames the PASS line; leave the other two messages' meaning intact and change only the word `fresh-gate` to `gate`).

## Requirements
### Functional
- **R1. The queue gate runs the full list.** `cycle.sh` runs the item's gate with `GATE_STEPS=gate-steps.txt` set explicitly on that command (the `( export ...; ... )` subshell), regardless of what `env.sh` exported. `env.sh`'s default is NOT changed (other tools may rely on it).
- **R2. The log says what ran.** Before the gate starts, `cycle.sh` logs `gate steps: <label, label, ...>` read from the list actually used, and the verdict line reads `gate PASS (<N> steps)` with the real count (today's "fresh-gate PASS" is wrong for a routine run and is replaced).
- **R3. The time bound is kept.** `GATE_TIMEOUT` is unchanged; a full gate that outlives it still ends as `gate-timeout`, never as a pass.
- **R4. A selector, testable without cargo.** A new script `scripts/autonomy/gate-env.sh`, sourced by `cycle.sh`, defines `gate_steps_file` (prints the list path for a queue item: always `gate-steps.txt`) and `gate_step_labels <file>` (prints the labels of a list, one per line, ignoring blank and `#` lines). `cycle.sh` uses both.
- **R5. A test with controls.** New `scripts/autonomy/tests/gate-steps.test.sh` asserts, each as an `ok`/`FAIL` line: (a) `gate_steps_file` returns `gate-steps.txt` even when `GATE_STEPS=gate-steps-quick.txt` is in the environment (the case today); (b) the labels of `gate-steps.txt` include `lint-observability`, `test (lib/bins/tests)` and `coverage-floor`; (c) CONTROL: the labels of `gate-steps-quick.txt` do NOT include `lint-observability` (so the check can tell the two lists apart); (d) `cycle.sh` contains no `quality-gate.sh --routine` invocation that lacks `GATE_STEPS=gate-steps.txt` on the same command; (e) CONTROL for (d): a planted copy of the line without the variable makes the same predicate fail; (f) a blank or comment-only list yields zero labels. It prints a final `N ok, 0 FAIL` and exits 0 only if every assertion passed.

### Non-Functional
- Shell only (bash, `set -u`); no Python call is added (the existing `python3 scripts/autonomy/state.py` calls in `cycle.sh` are not touched by this spec). No hardcoded absolute path in the new files beyond what `cycle.sh` already uses.
- `bash -n` passes for every edited or new script. `scripts/autonomy/selftest.sh` still passes if it ran before (run it, report the result, do not edit it).
- Do not edit `gate-steps.txt`, `gate-steps-quick.txt` or `quality-gate.sh`.

## TDD Contract
| Test (an `ok` line of `tests/gate-steps.test.sh`) | Expected |
|---|---|
| `gate_steps_file ignores GATE_STEPS from the environment` | prints `gate-steps.txt` with `GATE_STEPS=gate-steps-quick.txt` exported |
| `the full list names the three steps the quick list lacks` | `lint-observability`, `test (lib/bins/tests)`, `coverage-floor` present |
| `CONTROL: the quick list lacks lint-observability` | absent from `gate-steps-quick.txt` |
| `cycle.sh gates with the full list` | no bare `quality-gate.sh --routine` command in `cycle.sh` |
| `CONTROL: the predicate fails on a planted bare line` | the same predicate reports a violation on a temp copy |
| `a comment-only list has zero labels` | `gate_step_labels` prints nothing |

**The requirement a plausible implementation satisfies only in appearance:** editing `env.sh`'s default to the full list. That passes a naive check, changes every other tool that sources it, and leaves `cycle.sh` fragile to any future `env.sh` edit. The assertion (a) exports the quick list into the environment and requires the queue's selector to ignore it.

## Exit Criteria
- [ ] `test -f scripts/autonomy/gate-env.sh && test -f scripts/autonomy/tests/gate-steps.test.sh`
- [ ] `bash scripts/autonomy/tests/gate-steps.test.sh | tail -1 | grep -q " 0 FAIL"`
- [ ] `bash scripts/autonomy/tests/gate-steps.test.sh > /dev/null`
- [ ] `bash -n scripts/autonomy/cycle.sh && bash -n scripts/autonomy/gate-env.sh`
- [ ] `grep -q "gate-env.sh" scripts/autonomy/cycle.sh`
- [ ] `grep -q "GATE_STEPS=gate-steps.txt" scripts/autonomy/cycle.sh`
- [ ] `git diff --quiet -- gate-steps.txt gate-steps-quick.txt quality-gate.sh scripts/autonomy/env.sh`

## Guardrails
- Do NOT change `gate-steps.txt`, `gate-steps-quick.txt`, `quality-gate.sh` or `scripts/autonomy/env.sh`.
- Do NOT run the queue, `cycle.sh` itself, or a gate from the test: the test reads files and sources the helper only.
- Do NOT commit.

## Follow-ups (not this spec)
The same gap at `head-builds.sh` (it checks that HEAD builds, not that it passes the lints); a periodic `--fresh` release-boundary gate; moving the gate into the generic graph as a `qgate` sub-graph.
