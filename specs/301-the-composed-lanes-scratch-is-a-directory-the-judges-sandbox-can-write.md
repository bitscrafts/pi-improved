# Spec 301: the composed lane's scratch is a directory the judge's sandbox can write, and the preflight proves it through the daemon

## Overview
**Project**: /projects/pi-improved
**Phase**: production — composability, lane hardening (retest of A161 after specs 298-300)
Spec 298 added a container preflight (`mkdir -p /projects/_tmp && test -w /projects/_tmp`). The retest of A161 (run r000000000452, code coder 437 s, exit 0) was refused at `verify` AGAIN, 7/8, on the same
criterion (`scripts/checks/live-orphan-count.sh`), so 298's preflight passed (`PREFLIGHT: container scratch /projects/_tmp ok, uv ok`) while the judge failed. MEASURED cause: the judge runs each
criterion through the daemon (`judge_spec.rs run_all` -> `ctx.proc().run`), and the production daemon runs with landlock profile `workspace` ("WRITES confined to the workspace, store dir, runtime dir,
$TMPDIR ..."). Probe (`/projects/_tmp/probe-judge-sandbox.sh`, run r000000000454, no agent, nothing spent): a criterion `mkdir /projects/_tmp/probe-N` is REFUSED (`crit=2/3 fail=mkdir /projects/_tmp/probe-N`),
a criterion `mkdir -p _tmp/probe-N` in the workspace SUCCEEDS. The preflight ran under `podman exec`, outside the sandbox, so it proved nothing about the judge.

## Requirements
### Functional
- R1: the scratch the worker brief names is `_tmp` INSIDE the repository (`$REPO/_tmp`, i.e. `/projects/pi-improved/_tmp` in the container, which the compose file mounts and which is gitignored), not `/projects/_tmp`. The brief says "Scratch you create goes under _tmp/ in the repository" and the lane's `SCRATCH` variable is that path.
- R2: the preflight no longer tests the shell's write access. It runs a ONE-NODE JUDGE through the production daemon: a throwaway spec in a scratch git repo under `$REPO/_tmp/devbox-test/` whose single criterion is `mkdir -p <SCRATCH-relative>/preflight-<ts> && test -d <SCRATCH-relative>/preflight-<ts>`, using the tracked graph `graphs/preflight.toml` (new, one `judge` node, ok -> `done`, fail -> `refuse`), and requires the run to end `succeeded`. This is the same confinement the real judge has. On a `failed` run it prints `ABORT: container preflight failed: the daemon's judge cannot write <SCRATCH>` plus the trace line and exits 2 BEFORE the real run starts. A `--dry-run` still prints `PREFLIGHT: skipped (dry run)`.
- R3: `graphs/preflight.toml` is copied to the daemon's graph dir with the other two graphs; the lane's pin check also refuses a missing file.
- R4: keep spec 298's `uv` check (it is still needed: a criterion may call `uv`), run inside the same `podman exec` as before.

### Non-Functional
- The preflight costs no agent and no spend (a judge only). Test via the existing stub mechanism (`PII_SSH`, `PII_DAEMON_SH`); the stub answers the preflight run's `run start` and `run get`.

## Verified Premises
- **CONFIRMED** (2026-10-27) `pii health --json` -> `"sandbox_profile":"workspace"`; `crates/pii-tools/src/sandbox.rs:44` states the writes the profile allows.
- **CONFIRMED** probe run r000000000454: `5 verify ... crit=2/3 fail=mkdir /projects/_tmp/probe-1793136863`; the host sees `/projects/pi-improved/_tmp/devbox-test/judge-probe-1793136863/repo/_tmp/probe-1793136863` created; the container has no `/projects/_tmp/probe-1793136863`.
- **CONFIRMED** A161's own spec (297) names `/projects/_tmp` only as the default of `LIVE_ORPHAN_SCRATCH` in `scripts/checks/live-orphan-count.sh:17`; the script honours the variable.
- **INFERRED** the judge criterion's environment (`TMPDIR` unset -> `/tmp`, also unwritable under landlock) means a criterion that needs scratch must use a path inside the workspace; verify with the probe in TDD.

## Architecture
```
lane (spec) -> pin check (3 graphs) -> PREFLIGHT: uv check (podman exec) + one-node judge run (daemon, sandboxed) -> real run
```

## TDD Contract
| Test (scripts/autonomy/tests/lane-preflight.test.sh, extended, existing assertions untouched) | Input | Expected |
|---|---|---|
| `brief_names_the_repo_scratch` | the brief the lane writes | contains `_tmp/` under the repository wording and NOT `/projects/_tmp` |
| `preflight_runs_a_judge_through_the_daemon` | stub ssh records argv | a `run start preflight` precedes the real `run start spec`, and a `run get` of the preflight run |
| `CONTROL_a_failed_preflight_judge_aborts` | stub answers the preflight run `failed` | exit 2, `ABORT: container preflight failed`, no `run start spec` |
| `preflight_graph_exists_and_is_copied` | the tracked `graphs/preflight.toml` | present, one judge node, copied to the daemon graph dir (argv shows the cp) |
| existing 298 tests | unchanged | still `ok` (the stub grows a branch, no assertion changes) |

## Exit Criteria
- [ ] `bash scripts/autonomy/tests/lane-preflight.test.sh 2>&1 | tail -1 | grep -q "FAIL: 0"`
- [ ] `test "$(bash scripts/autonomy/tests/lane-preflight.test.sh 2>&1 | grep -c '^ok')" -ge 12`
- [ ] `test -f graphs/preflight.toml && test "$(grep -c 'kind = "judge"' graphs/preflight.toml)" = 1`
- [ ] `bash scripts/autonomy/tests/lane-graph-select.test.sh 2>&1 | tail -1 | grep -q "FAIL: 0"`
- [ ] `bash scripts/autonomy/tests/lane-wait-cancels.test.sh 2>&1 | tail -1 | grep -q "FAIL: 0"`
- [ ] `bash scripts/autonomy/tests/lane-facts.test.sh 2>&1 | tail -1 | grep -q "FAIL: 0"`
- [ ] `bash -n scripts/run-batchC-pii.sh`
- [ ] `git diff --quiet -- crates scripts/autonomy/env.sh scripts/autonomy/cycle.sh graphs/spec.toml graphs/docs.toml`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

## Files
- `scripts/run-batchC-pii.sh`
- `graphs/preflight.toml`
- `scripts/autonomy/tests/lane-preflight.test.sh`

## Guardrails
- Do NOT edit `crates/`, `cycle.sh`, `env.sh`, `graphs/spec.toml`, `graphs/docs.toml` or an existing assertion. No real ssh, daemon or agent in a test.
- The appearance-only implementation: a preflight that still runs under `podman exec` (outside the sandbox) and passes, which is exactly 298. The test that catches it: `CONTROL_a_failed_preflight_judge_aborts` can only go red if the verdict comes from the daemon's run status.
- Mutations the tests must kill: the preflight judge run not required to be `succeeded`; the brief naming `/projects/_tmp`; the preflight after the real `run start`; `graphs/preflight.toml` not copied; a failed preflight not aborting.
