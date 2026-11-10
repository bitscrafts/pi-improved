# Spec 304: the commit's lane comes from the run that changed the tree, and it survives a resume at verify

## Overview
**Project**: <repo>
**Phase**: production — lane hardening (spec 303's Follow-up, MEASURED 2026-10-28 00:44Z on A167's own commit)
Spec 303 made a queue row keep its lane on a resume and made the cycle append `Lane: spec|coder` to its commit. A167's commit `275c2af` carries **no** `Lane:` line, though its worker log `_tmp/claude-batchC/303-a-resumed-queue-row-keeps-its-lane-and-its-commit-says-which.1793146913.log` contains `LANE-GRAPH: coder` — because `cycle.sh:260` assigns `WLOG` inside the dispatch branch only, so a commit produced after a phase-marker resume (`SKIP_DISPATCH`) reads an empty `WLOG` and the trailer is skipped silently.
The same line has a second hole, measured the same night: the newest log belonged to run `r000000000466`, a 5-second coder-lane run that changed nothing (`tree_changed=no`, `cost_usd=0.000000`), while the tree the commit carried was produced by `r000000000465` in the spec lane. With `WLOG` fixed alone the trailer would have been `Lane: coder` — a false statement about the commit, which is worse than no trailer.

## Requirements
### Functional
- R1: the lane a ran worker actually used is recorded on the row: after a dispatch, the cycle parses the `LANE-GRAPH:` line of the log that run wrote and stores it as `lane=<spec|coder>` in the row's state (`state.py set`, the atomic path already used for `session`/`last`). The loop event `loop.dispatch` gains the same value.
- R2: a run that did not change the tree does not overwrite the record. The lane is written only when the run's own facts say `tree_changed=yes`; a run that changed nothing (`tree_changed=no`, as `r000000000466` did) leaves the previous record standing, so a commit cannot be attributed to a run that produced nothing.
- R3: the commit's trailer is read from, in order, (a) the recorded `lane` in the row's state, (b) the `LANE-GRAPH:` line of the worker log the commit step resolves (`WLOG`), (c) nothing. `WLOG` is resolved **before** the dispatch branch — not inside it — so the phase-marker-resume path (`SKIP_DISPATCH`, which produced `275c2af`) reaches the commit step with the same handle the dispatch path has.
- R4: no record and no `LANE-GRAPH:` line ⇒ no trailer (unchanged from 303; an invented lane is worse than none).
- R5: `state.py list` and `get` show the lane (no new file, no schema change: `set` already accepts arbitrary keys and the state is one JSON object per id).

### Non-Functional
- Shell + the existing `state.py`; `scripts/autonomy/cycle.sh` is the only production file edited. Tests use the existing lane-trailer harness shape (`cycle-lane-trailer.test.sh` extracts the `# lane-trailer:begin` … `# lane-trailer:end` block and runs it). Scratch under `_tmp/` inside the repository.

## Verified Premises
- **CONFIRMED** (2026-10-28 00:44Z): `git log -1 --format=%B 275c2af` has no `Lane:` line; `grep -a '^LANE-GRAPH: ' …303-….1793146913.log` prints `LANE-GRAPH: coder`.
- **CONFIRMED**: `cycle.sh:260` — `WLOG=$(ls -t _tmp/claude-batchC/"$N"-*.log …)` sits inside the worker branch, after `if [ -n "$SKIP_DISPATCH" ]; then NEWSID="$RSID"; else …`; the `SKIP_DISPATCH` path never assigns it.
- **CONFIRMED**: run `r000000000466`'s cycle log records `FACTS : rc=1 tree_changed=no quota_exhausted=no cost_usd=0.000000` and `== tree after: 9 dirty paths` — i.e. it produced no tree of its own; the same file's `LANE-GRAPH: coder` line is the newest for that spec number.
- **CONFIRMED**: `state.py set <id> k=v` writes any key through a temp file + `os.rename`; `get` returns it.

## TDD Contract
| Test (`scripts/autonomy/tests/lane-trailer-from-state.test.sh`) | Input | Expected |
|---|---|---|
| `trailer_from_the_recorded_lane` | the block run with an empty `WLOG` and a state row carrying `lane=spec` | the message ends with `Lane: spec` (this is `275c2af`'s shape: resume at verify) |
| `the_log_wins_when_no_record_exists` | no `lane` in the state, a log with `LANE-GRAPH: coder` | `Lane: coder` |
| `CONTROL_no_record_no_log_no_trailer` | neither | no `Lane:` line |
| `a_no_op_run_does_not_overwrite_the_record` | the dispatch-side writer run twice: `tree_changed=yes lane=spec`, then `tree_changed=no lane=coder` | the record stays `spec` |
| `the_dispatch_side_records_the_lane` | the writer run once with `tree_changed=yes` and a log line `LANE-GRAPH: spec` | the row's state holds `lane=spec` |
| `LAST_lane_line_wins` | a log with `coder` then `spec`, no record | `Lane: spec` (spec 303's rule, still true) |

## Exit Criteria
- [ ] `bash scripts/autonomy/tests/lane-trailer-from-state.test.sh 2>&1 | tail -1 | grep -q "FAIL: 0"`
- [ ] `test "$(bash scripts/autonomy/tests/lane-trailer-from-state.test.sh 2>&1 | grep -c '^ok')" -ge 6`
- [ ] `bash scripts/autonomy/tests/cycle-lane-trailer.test.sh 2>&1 | tail -1 | grep -q "FAIL: 0"`
- [ ] `awk '/# lane-trailer:begin/,/# lane-trailer:end/' scripts/autonomy/cycle.sh | grep -q 'state.py get'`
- [ ] `bash -n scripts/autonomy/cycle.sh`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

## Files
- `scripts/autonomy/cycle.sh`
- `scripts/autonomy/tests/lane-trailer-from-state.test.sh`

## Guardrails
- Only `scripts/autonomy/cycle.sh` changes in production: the `WLOG` resolution (it moves above the dispatch branch), the dispatch-side lane record, and the `# lane-trailer:begin` … `# lane-trailer:end` block. `tick.sh`, `state.py`, `env.sh` and the lane script are NOT edited.
- Do not invent a lane. Every trailer must be attributable to a recorded run or a log line the commit step actually read; when neither exists the trailer stays absent (R4). A test that passes by hard-coding `spec` in the block is the failure mode this spec exists to prevent.
- `cycle.sh` runs under a live loop: the test harness extracts the block, it does not execute the cycle. Never edit `cycle.sh` while a cycle holds `_tmp/autonomy/lock` (DO-NOT-REPEAT 2.15).
- The state write must go through `state.py set` (atomic); a direct write of `state.json` is refused by review.
- Mutations the tests must kill: the state fallback removed; the no-op guard removed (`tree_changed` ignored); the record written before the log is parsed; the trailer read from the FIRST `LANE-GRAPH:` line.

## Follow-up
- The lane knob `_tmp/autonomy/lane-graph` read `coder` at 2026-10-28 00:19:08Z, so no run this night used the composed lane. Whoever flips it back to `spec` should re-run `scripts/archive/composed-real-run.sh` once — the lane's last real-agent proof predates specs 303/304.

## Closure
(pending — filled by the orchestrator after the item's criteria, gate and mutation review)
