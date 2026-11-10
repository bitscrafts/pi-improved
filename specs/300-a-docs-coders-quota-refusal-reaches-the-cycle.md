# Spec 300: a docs coder's quota refusal reaches the cycle, and the cycle's requeue path prints its outcome

## Overview
**Project**: <repo>
**Phase**: production — composability, lane hardening (Opus review 2026-10-27, findings F3 and F8)
F3: in the composed lane only the FIRST `coder.exited` (the host's `code` coder) is parsed for `quota_exhausted`, and only that one writes the CLI-shaped line
`You've hit your session limit · resets <T> (UTC)` to the worker log. The docs coder's facts go to stdout (`DOCS-FACTS`), which `scripts/autonomy/window.sh` does not read (it reads the worker log). So a
refusal by the docs coder becomes `worker-rc=1` then `needs-review`, and the row waits for a person instead of being requeued at the window's reset. The engine side is proved (`composed-quota-check.sh`);
the lane side is not.
F8: `scripts/autonomy/cycle.sh:290` prints `OUTCOME=requeued-quota ITEM=$ITEM SESSION=$RECOVER` under `set -u`, and `RECOVER` is assigned only on the dirty-tree branch (`:152`). On the normal requeue path the
script dies on an unbound variable after writing the state, so no `OUTCOME=` line is printed (and the phone alarm reads that line). Affects both lanes.

## Requirements
### Functional
- R1: in the spec lane the quota line is written when EITHER `coder.exited` reports `quota_exhausted":"yes"`. The reset time is taken from the refusing coder's own `quota_reset`; the line is the same text as today, appended to the worker log and printed.
- R2: the `DOCS-FACTS` line gains nothing new, but the refusing coder is named in one extra line `QUOTA    : refused by the <host|docs> coder` printed with the quota line.
- R3: `cycle.sh:290` prints `SESSION=${RECOVER:-}`. No other change to `cycle.sh`.

### Non-Functional
- Shell only. Production files edited: `scripts/run-batchC-pii.sh`, `scripts/autonomy/cycle.sh` (the one expression). Tests use a stub `ssh` first on PATH answering from fixtures captured from the real log shape (see Verified Premises); no daemon, no agent.

## Verified Premises
- **CONFIRMED** (2026-10-27) `run-batchC-pii.sh:218-232` parses `EXITED` (first line) and `DOCS_EXITED` (second line) but only `QX=$(fact "$EXITED" quota_exhausted)` feeds the quota line.
- **CONFIRMED** `scripts/autonomy/window.sh:17-18` parses the worker log, not stdout.
- **CONFIRMED** `cycle.sh:152` is the only assignment of `RECOVER`; `:290` uses it; `set -u` is on at `:33`.
- **CONFIRMED** the real `coder.exited` journal line shape is `{"kind":"coder.exited", ... "quota_exhausted":"yes","quota_reset":"resets 8:20pm (UTC)" ...}` (captured by `scripts/archive/composed-quota-check.sh`; its fixtures write exactly this).

## TDD Contract
| Test (scripts/autonomy/tests/lane-facts.test.sh) | Input | Expected |
|---|---|---|
| `docs_quota_refusal_reaches_the_worker_log` | stub pii.log: code `quota_exhausted":"no"`, docs `"yes"` with `resets 8:20pm (UTC)` | the worker log holds `You've hit your session limit · resets 8:20pm (UTC)` and `window.sh` parses an epoch from it |
| `host_quota_refusal_still_reaches_the_log` | code `"yes"`, docs `"no"` | same line, `QUOTA    : refused by the host coder` |
| `CONTROL_no_refusal_writes_no_quota_line` | both `"no"` | no `session limit` line |
| `CONTROL_a_docs_coder_that_only_read_the_phrase_is_not_a_refusal` | docs `quota_exhausted":"no"` while its output text contains the phrase | no line |
| `requeue_prints_its_outcome` (scripts/autonomy/tests/cycle-requeue-outcome.test.sh) | run the real `cycle.sh` requeue branch with stub worker log `resets 8:20pm (UTC)`, rc 0, normal (not dirty-tree) path | stdout contains `OUTCOME=requeued-quota`, exit 0 |
| `CONTROL_the_old_expression_dies` | the same test against a copy of cycle.sh with `$RECOVER` restored | the test is red (unbound variable) |

## Exit Criteria
- [ ] `bash scripts/autonomy/tests/lane-facts.test.sh 2>&1 | tail -1 | grep -q "FAIL: 0"`
- [ ] `test "$(bash scripts/autonomy/tests/lane-facts.test.sh 2>&1 | grep -c '^ok')" -ge 4`
- [ ] `bash scripts/autonomy/tests/cycle-requeue-outcome.test.sh 2>&1 | tail -1 | grep -q "FAIL: 0"`
- [ ] `bash scripts/autonomy/tests/lane-preflight.test.sh 2>&1 | tail -1 | grep -q "FAIL: 0"`
- [ ] `bash scripts/autonomy/tests/lane-wait-cancels.test.sh 2>&1 | tail -1 | grep -q "FAIL: 0"`
- [ ] `bash scripts/autonomy/tests/lane-graph-select.test.sh 2>&1 | tail -1 | grep -q "FAIL: 0"`
- [ ] `bash -n scripts/run-batchC-pii.sh`
- [ ] `bash -n scripts/autonomy/cycle.sh`
- [ ] `git diff --quiet -- graphs crates scripts/autonomy/env.sh`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

## Files
- `scripts/run-batchC-pii.sh`
- `scripts/autonomy/cycle.sh`
- `scripts/autonomy/tests/lane-facts.test.sh`
- `scripts/autonomy/tests/cycle-requeue-outcome.test.sh`

## Guardrails
- Do NOT edit `graphs/`, `crates/`, `env.sh`, `window.sh` or an existing assertion. cycle.sh: change ONLY the one expression in R3. No real ssh, daemon, agent or quota spend in a test.
- Mutations the tests must kill: only the first coder parsed; the docs coder's `quota_reset` replaced by the host's; the line written when both are `no`; the refusing coder misnamed; `${RECOVER:-}` reverted.
