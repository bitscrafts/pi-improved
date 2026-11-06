# Spec: a skipped gate leaves the row where it was

## Overview
Measured twice tonight: a row reached `needs-review` with `OUTCOME=gate-skipped-disk` and had **never been
gated**. In `scripts/autonomy/cycle.sh` the disk-skip path sets

    status=needs-review attempts=$((ATTEMPTS+1)) last=gate-skipped-disk

so a skip is recorded as if the work had advanced: the row looks finished, the attempt counter climbs
toward the park threshold, and parking quarantines the tree. A skip that moves a row forward is a skip
that reports success — the same shape as `gate-skipped-disk` with 91 GB free, and as a check that is not
a lock.

**Project**: /projects/pi-improved
**Phase**: operations (the dispatcher)

## Requirements
### Functional
- R1 a skipped gate leaves the row `pending` and does NOT increment `attempts`: nothing was tried, so
  nothing was attempted.
- R2 the reason is written where an operator reads it (`last=gate-skipped:<why>`), and the OUTCOME line
  says the row was not advanced.
- R3 a missing writable directory is reported as WIRING, not as a disk shortage: `disk-floor.sh` exits 4
  for `MISSING:` and cycle.sh names the path.
- R4 every driver that can skip a gate uses the same rule (cycle.sh today; the drain if it gains one).

### Non-Functional
- R5 the row is offered again on the next firing once the cause is repaired — no operator action needed.

## TDD Contract
| Test name | Input | Expected |
|---|---|---|
| `a_skipped_gate_leaves_the_row_pending` | a row, disk below the floor | status stays `pending`, attempts unchanged |
| `a_missing_write_dir_is_wiring` | `GATE_WRITE_DIRS` naming an absent path | exit 4, message names the path |
| `CONTROL_a_green_gate_still_advances` | room on disk, gate passes | status advances, attempts unchanged |

## Exit Criteria
- [ ] `bash scripts/autonomy/tests/skip-is-not-a-pass.test.sh 2>&1 | tail -1 | grep -q '0 FAIL'`
- [ ] `test "$(grep -c 'attempts=$((ATTEMPTS+1)) last=gate-skipped' scripts/autonomy/cycle.sh)" = 0`
- [ ] `cargo test -q -p xtask --test gate_skip_is_not_a_pass 2>&1 | grep -qE 'test result: ok\. [1-9][0-9]* passed'`
- [ ] `! grep -q 'status=needs-review attempts=$((ATTEMPTS+1)) last=gate-skipped' scripts/autonomy/cycle.sh`

Why this section has fewer criteria than the requirements imply: `sdd`'s contract step runs every
criterion BEFORE any code exists and refuses a row if one already passes (measured 2026-11-05: eight
dispatches refused, none spent). A criterion that passes today is a regression guard, not a test of the
new behaviour, so the guards are stated under Guardrails and enforced by the gate.

## Guardrails
- Do NOT keep `needs-review` for a skipped gate: that is the defect, not the label.
- Do NOT make the disk floor lower to avoid the skip.
- Do NOT delete the row's work when it yields; a skip changes nothing on disk.

## Files
- `scripts/autonomy/cycle.sh`
- `scripts/autonomy/disk-floor.sh`
- `scripts/autonomy/tests/disk-floor.test.sh`
- `scripts/autonomy/tests/skip-is-not-a-pass.test.sh`
- `xtask/tests/gate_skip_is_not_a_pass.rs`
