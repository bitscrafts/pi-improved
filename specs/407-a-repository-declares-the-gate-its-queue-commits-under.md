# Spec: a repository declares the gate its queue commits under
#: depends=405 status=pending block=-

## Overview
Spec 405 lets each repository run its own queue. The one thing that still belongs to pi-improved is the gate: the cycle commits a row only after
`bash quality-gate.sh --routine` passes with `GATE_STEPS=gate-steps.txt`, and it counts and names the steps from `gate-steps.txt` (`cycle.sh` lines 438
and 442, `gate-env.sh`). A repository whose gate is `cargo xtask gate` (laya-rs, measured 2026-11-10) has no `quality-gate.sh`, so its queue would
refuse every commit, or worse, be pointed at a gate that does not exist and pass vacuously.

Two other callers bake in a Rust workspace: `head-builds.sh` and `pre-commit-builds.sh` run `cargo check -q --workspace`. That is right for a Cargo
repository and is kept as the default; a repository can say it differently.

Decision: a repository states its gate in one small file, `.pii/gate`, a `key = value` file the shell reads with no interpreter. Absent file = today's behaviour exactly,
so pi-improved changes nothing. The file names commands, not scripts of the queue, so the gate stays the repository's own.

**Project**: <repo>
**Phase**: the queue serves more than one repository
**Depends on**: 405

## Requirements
- R1 `.pii/gate` at the repository root, lines `key = value`, `#` comments and blank lines ignored. Keys: `command` (the gate; run from the repo root, exit 0 = pass),
  `steps_file` (optional: a list the cycle uses to count and name the steps in its log, `label|command` lines as `gate-steps.txt`), `build_check` (optional: the
  fast "does the tree build" command; default `cargo check -q --workspace`), `min_free_mb` (optional integer; default 12000), `timeout_secs` (optional integer; default 3600).
- R2 `scripts/autonomy/gate-env.sh` gains `gate_command`, `gate_build_check`, `gate_min_free_mb` and `gate_timeout`. Each reads `.pii/gate` of `$PII_ROOT` and falls back to
  today's value when the file or the key is absent: `gate_command` is `GATE_STEPS=gate-steps.txt QG_SKIP_COVERAGE=0 bash quality-gate.sh --routine`.
- R3 `cycle.sh` runs `gate_command` where it now has the literal `quality-gate.sh --routine`, under `gate_timeout` and with `FRESH_GATE_MIN_FREE_MB=$(gate_min_free_mb)`.
  The coverage rule (spec 266: a gate that never measured coverage is not a pass) applies only when the repository's gate has a coverage step, which is when
  its `steps_file` names `coverage-floor`; a repository with another gate is not held to a pi-improved log line it can never print.
- R4 A key that is not one of the five, a line that is not `key = value`, an empty `command`, or a non-integer `min_free_mb`/`timeout_secs` is refused at the start of
  the cycle: `OUTCOME=plan-error`, exit 4, nothing spent, and the message names the file and the line.
- R5 The gate command is never built from a row, an issue or a spec: only from `.pii/gate` in the repository the queue belongs to. A `command` is run through `bash -c` once.
- R6 `head-builds.sh` and `pre-commit-builds.sh` run `gate_build_check` inside their scratch export in place of the literal `cargo check`, and keep `cargo metadata --locked`
  only when the default is in force.

## Architecture
```
.pii/gate (optional)  --read by-->  gate-env.sh:  gate_command | gate_build_check | gate_min_free_mb | gate_timeout   (absent => today's values)
cycle.sh      step 5:  ( cd ROOT; timeout $(gate_timeout) bash -c "$(gate_command)" )   +  coverage rule only if steps_file names coverage-floor
head-builds.sh / pre-commit-builds.sh:   in the export:  bash -c "$(gate_build_check)"
```
Key decisions:
- A plain file with five keys, read by shell. Rejected: reading `pii.toml` (the daemon's file; the queue must work with no daemon, and a repository's gate is not a daemon setting);
  and a per-row gate column (53 readers, and the gate is a property of the repository, not of a row).
- The default is today's literal, produced by the same function. Rejected: leaving the literal in `cycle.sh` and adding a branch; two paths means pi-improved can silently stop
  using the one it relies on. The control test proves the default equals the old command byte for byte.
- `command` is a string run once by `bash -c`, from the repository's own file. Rejected: arguments from the plan or the issue; that is how an issue becomes an instruction.

## TDD Contract
New test file `scripts/autonomy/tests/repo-gate.test.sh` (sources `gate-env.sh` with `PII_ROOT` pointing at a scratch directory under `_tmp/`; never runs a gate).

| Test Name | Input | Expected Output |
|---|---|---|
| `without_a_file_the_gate_is_the_one_pi_improved_had` | no `.pii/gate` | `gate_command` equals `GATE_STEPS=gate-steps.txt QG_SKIP_COVERAGE=0 bash quality-gate.sh --routine`; `gate_build_check` = `cargo check -q --workspace`; 12000; 3600 |
| `a_declared_command_replaces_the_gate` | `command = cargo xtask gate` | `gate_command` is that string |
| `CONTROL_an_absent_key_keeps_its_default` | only `command` set | `gate_min_free_mb` 12000, `gate_timeout` 3600, `gate_build_check` the default |
| `a_declared_timeout_and_floor_are_read` | `timeout_secs = 900`, `min_free_mb = 8000` | 900 and 8000 |
| `an_unknown_key_is_refused_naming_the_line` | `comand = x` | the check exits non-zero; the message names `.pii/gate` and line 1 |
| `a_non_integer_floor_is_refused` | `min_free_mb = lots` | refused, names the line |
| `an_empty_command_is_refused` | `command =` | refused |
| `comments_and_blank_lines_are_ignored` | a file of comments around one key | the key is read |
| `cycle_pins_no_literal_gate` | `cycle.sh` text | no line outside a comment contains `quality-gate.sh --routine`; it calls `gate_command` |
| `coverage_is_only_demanded_of_a_gate_that_has_it` | `steps_file` without `coverage-floor` | `gate_has_coverage` is false; with the step listed it is true; with no file it is true (today) |
| `CONTROL_the_cycle_refuses_a_malformed_file_before_the_lock` | scratch repo with a bad `.pii/gate`, `DRY=1 cycle.sh` | `OUTCOME=plan-error`, exit 4, no lock file, the fake `claude` not called |

Acceptance test: `without_a_file_the_gate_is_the_one_pi_improved_had`. The dangerous requirement is the default: a plausible implementation reads the file, passes every
declared-value row, and quietly changes what pi-improved itself is gated with (a missing `QG_SKIP_COVERAGE=0` re-enables the skip that spec 266 closed). The default row compares to the
old command exactly.

## Exit Criteria
### New in this row
Each is verified to FAIL before the change. A criterion that already passes is a Guardrail, never a criterion.
- [ ] `bash scripts/autonomy/tests/repo-gate.test.sh 2>&1 | grep -qE '^PASS: [1-9][0-9]* FAIL: 0$'`
- [ ] `! grep -n -E '^[^#]*quality-gate\.sh --routine' scripts/autonomy/cycle.sh`

## Guardrails
- Keep green (they pass today): `bash scripts/autonomy/tests/run-tests.sh`, `bash scripts/autonomy/tests/queue-root.test.sh`, `bash scripts/autonomy/tests/gate-steps.test.sh` (its text assertions about
  the routine gate are updated to name `gate_command`, never weakened).
- Do NOT change what pi-improved's own gate runs: `gate-steps.txt`, `quality-gate.sh`, `xtask` are untouched, and with no `.pii/gate` the cycle runs exactly the command it ran before.
- Do NOT add `.pii/gate` to pi-improved.
- Do NOT start a gate or a cycle from a test (`DRY=1` or a refusal before the probe only).
- A file over 450 lines does not get longer (`cycle.sh` is baselined): each change is an in-place swap.
- Every dirty path must be listed under Files: list a file before you touch it.

## Files
- `scripts/autonomy/gate-env.sh`
- `scripts/autonomy/cycle.sh`
- `scripts/autonomy/head-builds.sh`
- `scripts/autonomy/pre-commit-builds.sh`
- `scripts/autonomy/tests/repo-gate.test.sh`
- `scripts/autonomy/tests/gate-steps.test.sh`
- `scripts/autonomy/tests/gate-stamps-from-git.test.sh`
