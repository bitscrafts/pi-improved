# Spec 324: the gate runs the autonomy shell tests

## Overview

**Project**: /projects/pi-improved
**Phase**: autonomy lane / gate coverage

On 2026-10-31 `scripts/autonomy/tests/tick-message-worker-rc.test.sh` was found red — **and had been red
since 2026-10-30**: commit `8b1cb08` (spec 318 R4) changed `tick.sh` so that a verify failure is retried
(`retry:` key) and a gate failure re-gated (`regate:` key), and the test stayed on spec 313's "needs a hand"
sentence. It read *16 ok, 2 FAIL* for two days while every gate run in that window was green, because
`cargo test` never touches these files: they are shell, living under `scripts/autonomy/tests/`.

A test the gate cannot run is a comment with an exit code. The regression was not the stale sentence; it was
that nothing was watching. This spec puts the autonomy tests inside the gate, and gives the runner a control
that proves it can fail.

## Requirements

**R1 — a runner exists.** `scripts/autonomy/run-tests.sh [DIR]` runs every `DIR/*.test.sh` (default: its own
`tests/` directory) with a per-file timeout, prints one `ok <name> — <last line>` or
`FAIL <name> (rc=N) — <last line>` per file plus the failing file's last six lines, then a summary line, and
exits 0 only when every file passed.

**R2 — it fails closed.** A missing directory exits 2. A directory with **no** `*.test.sh` exits 2 and says
so: a runner that finds nothing must never report success.

**R3 — it can be seen to fail.** A control test (`gate-runs-the-autonomy-tests.test.sh`) runs the real runner
against scratch directories and asserts: a green directory exits 0; a directory containing one red test
exits **1**, names the file (`FAIL b (rc=1)`) and keeps that test's own last line in the output; a directory
that does not exist exits 2.

**R4 — the gate runs it.** Both step lists (`gate-steps.txt`, `gate-steps-quick.txt`) carry a step whose
command is `bash scripts/autonomy/run-tests.sh`, so the local gate and the `fresh-gate` export run the same
thing. The control asserts the step is present in both, so removing it fails a test.

**R6 — a test that cannot run says so.** A test may declare `# requires: <cap>[,<cap>]` in its first lines.
The runner runs it when the capability is available and prints
`SKIP <name> — requires <caps> (not available under <root>); it is NOT counted as a pass` otherwise. Today's
only capability is `dev-tree` (the tree under test is a git work tree): the five `lane-*` tests clone
`/projects/pi-improved` by absolute path, so in a clean export they would test the dev repo, not the export.
**An unknown requirement is refused (exit 2)**, so a typo can never silently skip a test, and a skip is never
counted as a pass.

**R7 — skips are visible.** The summary line carries the skipped count and names the skipped files, so a run
that skipped 5 of 28 cannot be mistaken for a run that ran them.

**R5 — the step runs in the export.** A `fresh-gate --worktree` run must show a PASS line for the new step;
that is the evidence the export carries the scripts and the step's command finds them.

## Files

- `scripts/autonomy/run-tests.sh`
- `scripts/autonomy/tests/gate-runs-the-autonomy-tests.test.sh`
- `gate-steps.txt`
- `gate-steps-quick.txt`

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| gate-lists-the-step | both step lists | one step per list whose command is the runner |
| a-green-directory-passes | scratch dir, one passing test | runner exits 0 |
| a-red-directory-fails | scratch dir, one failing test | runner exits 1, names the file, keeps its last line |
| a-missing-directory-is-refused | non-existent dir | runner exits 2 |
| the-real-directory-is-not-empty | `scripts/autonomy/tests` | at least 20 test files (a wrong path must not pass) |
| an-unavailable-requirement-is-skipped | stub declaring `dev-tree` under a tree without `.git` | exit 0, `SKIP`, counted skipped and not passed |
| a-satisfied-requirement-runs | same stub under a tree with `.git` | the test runs and passes |
| an-unknown-requirement-is-refused | stub declaring `dev-treey` | exit 2 |

## Exit Criteria

- [ ] `bash scripts/autonomy/tests/gate-runs-the-autonomy-tests.test.sh`
- [ ] `bash scripts/autonomy/run-tests.sh`
- [ ] `cargo run -q -p xtask -- fresh-gate --worktree 2>&1 | tail -1 | grep -q "fresh-gate: PASS"`
- [ ] `grep -q "autonomy (shell tests)|bash scripts/autonomy/run-tests.sh" gate-steps.txt`
- [ ] `grep -q "autonomy (shell tests)|bash scripts/autonomy/run-tests.sh" gate-steps-quick.txt`

## Guardrails

- Do NOT make the runner skip a file it cannot execute: an unreadable or non-executable test is a FAIL.
- Do NOT let the runner pass on an empty directory, or on a directory it was pointed at by mistake.
- Do NOT weaken a test to make this step green: if a test is red, the code or the test is wrong — say which,
  with the measurement (that is how the tick-message test was settled on 2026-10-31).
- Do NOT put the runner's own control in a separate tree: it must live in `scripts/autonomy/tests/` so the
  runner exercises it too.
