# Spec 299: the composed lane waits as long as the graph allows, and cancels the run when it gives up

## Overview
**Project**: <repo>
**Phase**: production — composability, lane hardening (Opus review 2026-10-27, finding F2)
`scripts/run-batchC-pii.sh` polls the run for `MAXI = PII_NODE_TIMEOUT_MS/5000 + 120` iterations of (5 s + one ssh/podman call), about 65-75 minutes, while `graphs/spec.toml` allows
`max_wall_ms = 6_000_000` (100 minutes: code 50 + docs 20 + judges). When the poll expires the lane exits 1 WITHOUT cancelling: the engine never cancels a run because its client stopped
polling. The cycle then records `worker-rc=1`, releases the lock, and the next tick can dispatch another row onto the same tree while the old run's docs coder or judge is still writing to it:
two writers on one tree. `PII_NODE_TIMEOUT_MS` is also ignored by the spec lane's graph (`spec.toml` fixes the code node's timeout).

## Requirements
### Functional
- R1: in the spec lane the deadline is TIME-based, not iteration-based: `DEADLINE = max_wall_ms/1000 + 600` seconds read from the `max_wall_ms` line of `graphs/spec.toml` (a missing or non-numeric value aborts with exit 2 before anything starts), measured with bash `SECONDS`. The coder lane keeps its current loop untouched.
- R2: the lane prints `WAIT     : up to <DEADLINE>s (graph max_wall_ms <n> + 600)` before polling.
- R3: when the deadline passes with the status still not terminal, the lane runs `run cancel <RID> --kill --reason lane-wait-expired` in the container, prints `== <RID> still running after <n>s: cancelled by the lane`, appends the same line and the cancel's output to the worker log, then exits 1. Spec lane only.
- R4: the status words the loop recognises as terminal stay `succeeded|failed|refused|cancelled` (the engine serialises a cancel as `failed`; no behaviour change, only a comment saying so).

### Non-Functional
- Shell only. `run-batchC-pii.sh` is the only production file edited. Tests use a stub `ssh` first on PATH and `PII_POLL_SECONDS` (a new env var, default 5) so a test runs in seconds; the default behaviour is unchanged.

## Verified Premises
- **CONFIRMED** (2026-10-27) `run-batchC-pii.sh:205` `MAXI=$(( ${PII_NODE_TIMEOUT_MS:-3000000} / 5000 + 120 ))` and `:240` the lane exits with no cancel.
- **CONFIRMED** `graphs/spec.toml` carries `max_wall_ms = 6000000`; `crates/pii-graph/src/run/engine.rs:72` checks the wall only between nodes.
- **CONFIRMED** `pii run cancel <id> --kill --reason T` exists (`crates/pii-cli/src/cmd_sys.rs:192`) and was proved against a real run (`COMPOSED-FAULTS cancel: 7 pass`).

## TDD Contract
| Test (scripts/autonomy/tests/lane-wait-cancels.test.sh) | Input | Expected |
|---|---|---|
| `deadline_comes_from_the_graph_wall` | dry run, a copy of spec.toml with `max_wall_ms = 7000000` | the `WAIT` line says 7600 |
| `CONTROL_a_graph_without_a_wall_aborts` | spec.toml copy with the line removed | exit 2, nothing started |
| `a_run_that_outlives_the_deadline_is_cancelled` | stub ssh: status stays `running`, `PII_POLL_SECONDS=1`, a wall of 2000 ms | a `run cancel ... --kill --reason lane-wait-expired` argv recorded, exit 1, the log names the cancel |
| `CONTROL_a_run_that_finishes_is_not_cancelled` | stub: `succeeded` on the 2nd poll | no cancel argv recorded, exit 0 |
| `CONTROL_the_coder_lane_never_cancels` | `PII_LANE_GRAPH` unset, status stays `running` for MAXI iterations (`PII_NODE_TIMEOUT_MS=1000`) | exit 1, no cancel argv |

## Exit Criteria
- [ ] `bash scripts/autonomy/tests/lane-wait-cancels.test.sh 2>&1 | tail -1 | grep -q "FAIL: 0"`
- [ ] `test "$(bash scripts/autonomy/tests/lane-wait-cancels.test.sh 2>&1 | grep -c '^ok')" -ge 5`
- [ ] `bash scripts/autonomy/tests/lane-preflight.test.sh 2>&1 | tail -1 | grep -q "FAIL: 0"`
- [ ] `bash scripts/autonomy/tests/lane-graph-select.test.sh 2>&1 | tail -1 | grep -q "FAIL: 0"`
- [ ] `bash scripts/autonomy/tests/lane-graph-wall.test.sh 2>&1 | tail -1 | grep -q "FAIL: 0"`
- [ ] `bash -n scripts/run-batchC-pii.sh`
- [ ] `git diff --quiet -- graphs crates scripts/autonomy/env.sh scripts/autonomy/cycle.sh`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

## Files
- `scripts/run-batchC-pii.sh`
- `scripts/autonomy/tests/lane-wait-cancels.test.sh`

## Guardrails
- Do NOT edit `graphs/`, `crates/`, `cycle.sh`, `env.sh` or an existing assertion. No real ssh, daemon or agent in a test.
- Mutations the tests must kill: the deadline taken from `PII_NODE_TIMEOUT_MS`; no cancel on expiry; a cancel without `--kill`; a cancel issued for a run that finished; the coder lane cancelling; `+ 600` dropped.
