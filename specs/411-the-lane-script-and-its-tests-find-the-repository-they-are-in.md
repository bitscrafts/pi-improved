# Spec: the lane script and its tests find the repository they are in
#: depends=405 status=pending block=-

## Overview
A fresh clone of the repository at any other path fails three autonomy shell tests that pass at `/projects/pi-improved`: `lane-graph-wall`, `lane-preflight` and `lane-graph-knob`
(measured 2026-11-12: the same tests, the same environment, on the pushed tip at the real path, on a clone of the same tip elsewhere, and on the rewritten history elsewhere:
pass, fail, fail with identical counts). The full gate therefore cannot be proven from a clone, which is what a contributor or a CI runner has.

Two causes, read from the code, not guessed:
1. `scripts/run-batchC-pii.sh` sets `REPO=/projects/pi-improved` and `C=/projects/pi-improved/_tmp/devbox-test`. Run from a clone it still writes its graph under the real repository,
   while the test reads the graph under its own checkout, so `lane-graph-wall` reports "the script left no graph" and `lane-preflight` cannot find the preflight graph it expects copied.
2. `lane-graph-knob` case `default_path_sits_beside_the_control_file` requires the file `_tmp/autonomy/control` to exist. That file is gitignored runtime state that appears only once an
   operator pauses the queue, so a fresh checkout never has it, wherever it is.

Row 405 already taught the queue scripts to find their root from their own location. This row does the same for the one lane script those tests run, and makes the knob test compare
directories instead of requiring a runtime file.

**Project**: /projects/pi-improved
**Phase**: queue portability

## Requirements
1. `scripts/run-batchC-pii.sh` takes its repository root from `PII_ROOT` when set, else from its own location (`scripts/..`), and derives `C` from it. Run from `/projects/pi-improved` every
   value it computes is byte-identical to today's.
2. The path of the devbox's own mount, used inside the container (`podman exec -w /projects/pi-improved`), is NOT the repository root of the checkout and stays as it is.
3. `lane-graph-knob`'s default-path case compares the directory of the default lane file with the directory the queue's switch lives in (the one `scripts/autonomy/control.sh` names),
   and no longer needs the switch file to exist. If `control.sh` moved its switch, the case must fail.
4. A new test, `scripts/autonomy/tests/lane-scripts-find-their-root.test.sh`, exports the committed tree (`git archive HEAD`) into a scratch directory and runs the three tests there with the
   gate's own environment; all three must pass. Control: a copy of the lane script with the root hard-coded again must make `lane-graph-wall` fail in that scratch tree.
5. No other test is edited and no check is weakened: a test that passed still passes, and none is excused with a skip.

## Architecture
```
scripts/run-batchC-pii.sh   REPO=${PII_ROOT:-<script dir>/..}   C=$REPO/_tmp/devbox-test      (container path unchanged)
        │ run by
        ▼
lane-graph-wall / lane-preflight / lane-graph-knob  (read the graph the script wrote under THEIR checkout)

lane-scripts-find-their-root.test.sh:  git archive HEAD → scratch → the three tests pass there
                                       control: hard-coded root copy → lane-graph-wall fails there
```

## TDD Contract
| Test Name | Input | Expected Output |
|---|---|---|
| `the_three_lane_tests_pass_in_a_copy_of_the_tree` | `git archive HEAD` exported to a scratch dir, the three tests run there | all three exit 0 |
| `CONTROL_a_hard_coded_root_fails_in_the_copy` | the lane script with `REPO=/projects/pi-improved` put back, run in the copy | `lane-graph-wall` fails |
| `the_lane_script_keeps_its_values_at_the_real_path` | the script's resolved `REPO` and `C` when run from `/projects/pi-improved` | `/projects/pi-improved` and `/projects/pi-improved/_tmp/devbox-test` |
| `the_container_mount_path_is_unchanged` | `grep` of the `podman exec -w` line | still names `/projects/pi-improved` |
| `the_knob_default_is_compared_by_directory` | `lane-graph-knob` with no `_tmp/autonomy/control` file | the default-path case passes |
| `CONTROL_a_moved_switch_fails_the_knob_case` | `control.sh` pointed at another directory (a copy) | the default-path case fails |

## Exit Criteria
### New in this row
Each is verified to FAIL before the change. A criterion that already passes is a Guardrail, never a criterion.
- [ ] `bash scripts/autonomy/tests/lane-scripts-find-their-root.test.sh 2>&1 | grep -qE 'PASS: [1-9][0-9]* FAIL: 0'`
- [ ] `grep -q 'PII_ROOT' scripts/run-batchC-pii.sh && ! grep -q '^REPO=/projects/pi-improved' scripts/run-batchC-pii.sh`

## Guardrails
- Do NOT change the container path in `podman exec -w /projects/pi-improved`.
- Do NOT edit `lane-graph-wall.test.sh` or `lane-preflight.test.sh`; they must pass as written once the script finds its root.
- Do NOT skip, delete or weaken a test, and do NOT make a case pass by creating `_tmp/autonomy/control` in the checkout.
- Do NOT touch `/root/.pi`, any credential, or the other scripts under `scripts/` that name the real path (a later row).
- A file stays under 450 lines.

## Files
- `scripts/run-batchC-pii.sh`
- `scripts/autonomy/tests/lane-graph-knob.test.sh`
- `scripts/autonomy/tests/lane-scripts-find-their-root.test.sh`
