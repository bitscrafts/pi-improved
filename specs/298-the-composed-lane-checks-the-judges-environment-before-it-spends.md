# Spec 298: the composed lane checks the judge's environment before it spends, and logs why a run was refused

## Overview
**Project**: <repo>
**Phase**: production — composability, lane hardening (Opus review 2026-10-27, findings F1 and F4)
The composed lane's `verify`/`reverify` judges run the spec's `## Exit Criteria` INSIDE the devbox container, while the cycle checks the row's VERIFY on the host. The worker brief tells the
agent to put scratch under `<scratch>/`, which does not exist in the container (it mounts only `<repo>`). Measured: A161 (spec 297), run r000000000419, the coder worked
458 s and exited 0, then `verify` refused `crit=7/8` because `scripts/checks/live-orphan-count.sh` does `mktemp -d <scratch>/...` and the container answered `No such file or directory`.
Second defect on the same path: when a run is refused, the worker log holds `run explain`, which names no criterion, so the cycle records `worker-rc=1` with no reason.

## Requirements
### Functional
- R1: in the spec lane, before the daemon is asked to start the run and after the pin check, the lane runs ONE preflight in the container (`podman exec pii-devbox sh -c ...`): `mkdir -p <scratch>` and `test -w <scratch>` (the scratch directory the brief names), and `command -v uv`. It prints `PREFLIGHT: container scratch <scratch> ok, uv ok` on success. On failure it prints `ABORT: container preflight failed: <what>` and exits 2 BEFORE the run is started (nothing is spent). A `--dry-run` skips the container call and prints `PREFLIGHT: skipped (dry run)`.
- R2: the preflight command is built in ONE variable `PREFLIGHT_CMD` so the test can read it, and the scratch path in it is the SAME string the worker brief names (one variable, `SCRATCH=<scratch>`, used in both places).
- R3: when the run's final status is not `succeeded` (spec lane), the lane appends to the worker log, and prints, the lines of `run trace <RID>` that contain `crit=` or `fail=` under a header `== why the run was not accepted:`. If the trace holds none, it prints `== why the run was not accepted: (the trace names no criterion)`.

### Non-Functional
- Shell only. No change to the one-node coder lane (`PII_LANE_GRAPH` unset keeps today's behaviour byte for byte, proven by the existing `lane-graph-select.test.sh`).
- `scripts/run-batchC-pii.sh` is the only production file edited. A test must not start a daemon or spend anything: it puts a stub `ssh` first on PATH (a script that records its argv and answers from a fixture) and runs the real lane script.

## Verified Premises
- **CONFIRMED** (2026-10-27) in the container: `ls <scratch>` -> `No such file or directory`; `command -v python3` -> nothing; `command -v uv` -> `/usr/local/bin/uv`.
- **CONFIRMED** `ssh ... podman exec -w <repo> pii-devbox sh -c 'cd <repo> && bash scripts/checks/live-orphan-count.sh'` prints `LIVE-ORPHAN-COUNT: FAIL no scratch directory`.
- **CONFIRMED** `run trace r000000000419` prints `13 verify - ... crit=7/8 fail=bash scripts/checks/live-orphan-count.sh ...`.
- **CONFIRMED** `scripts/run-batchC-pii.sh:66` names `<scratch>/` in the brief; `:205-212` is the wait loop; `:215` is where the lane writes `explain` to the log.
- **INFERRED** the container's `<scratch>` created by the preflight lives in the container's writable layer only (the compose file says so) and so vanishes on `up`. That is fine: the preflight recreates it every dispatch.

## Architecture
```
run-batchC-pii.sh (LANE=spec)  pin check -> PREFLIGHT (R1/R2) -> daemon up -> run start -> wait -> final status
                                                                                         \-> not succeeded: trace -> log (R3)
```

## TDD Contract
| Test (scripts/autonomy/tests/lane-preflight.test.sh) | Input | Expected |
|---|---|---|
| `preflight_runs_before_the_run_starts` | stub ssh, spec lane | the preflight argv is recorded BEFORE any `run start` argv |
| `preflight_creates_the_scratch_the_brief_names` | the recorded preflight command and the brief file | both contain the same `<scratch>` string |
| `CONTROL_a_failing_preflight_aborts_with_nothing_started` | stub ssh answers rc=1 to the preflight | exit 2, `ABORT: container preflight failed`, no `run start` recorded |
| `dry_run_skips_the_preflight` | `--dry-run` | `PREFLIGHT: skipped (dry run)`, stub ssh never called |
| `coder_lane_has_no_preflight` | `PII_LANE_GRAPH` unset, dry run | no `PREFLIGHT:` line |
| `refused_run_logs_the_failing_criterion` | stub: final status failed, trace holds a `crit=7/8 fail=bash x.sh` line | the worker log and stdout contain `why the run was not accepted` and that line |
| `CONTROL_a_succeeded_run_logs_no_reason` | stub: succeeded | no `why the run was not accepted` line |
| `trace_without_a_criterion_says_so` | stub: failed, trace without `crit=` | the `(the trace names no criterion)` line |

## Exit Criteria
- [ ] `bash scripts/autonomy/tests/lane-preflight.test.sh 2>&1 | tail -1 | grep -q "FAIL: 0"`
- [ ] `test "$(bash scripts/autonomy/tests/lane-preflight.test.sh 2>&1 | grep -c '^ok')" -ge 8`
- [ ] `bash scripts/autonomy/tests/lane-graph-select.test.sh 2>&1 | tail -1 | grep -q "FAIL: 0"`
- [ ] `bash scripts/autonomy/tests/lane-graph-wall.test.sh 2>&1 | tail -1 | grep -q "FAIL: 0"`
- [ ] `bash scripts/autonomy/tests/lane-graph-knob.test.sh 2>&1 | tail -1 | grep -q "FAIL: 0"`
- [ ] `bash scripts/autonomy/tests/lane-input-matches-graph.test.sh 2>&1 | tail -1 | grep -q "FAIL: 0"`
- [ ] `bash -n scripts/run-batchC-pii.sh`
- [ ] `git diff --quiet -- graphs crates scripts/autonomy/env.sh scripts/autonomy/cycle.sh`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

## Files
- `scripts/run-batchC-pii.sh`
- `scripts/autonomy/tests/lane-preflight.test.sh`

## Guardrails
- Do NOT edit `graphs/`, `crates/`, `cycle.sh`, `env.sh` or an existing test assertion. Do NOT start a daemon, use the real ssh, or spawn an agent in any test. Scratch for a test goes under `_tmp/` and is removed only by a guarded script.
- Mutations the tests must kill: the preflight after `run start`; the brief naming a different scratch path than the preflight; a failing preflight that does not abort; the preflight in the coder lane; the reason printed for a succeeded run; the reason lines taken from `explain` instead of `trace`.
