# Spec 280: a row parked by a closed window is resumed when the window reopens

## Overview
**Project**: <repo>
**Phase**: production — unattended queue. On 2026-10-25 A142's worker was refused with `You've hit your session limit · resets 8:20pm (UTC)`. The cycle did the right things: it recorded the reset time, saved the worker's session id on the row (`session=f8b9331c…`) and stopped (`scripts/autonomy/cycle.sh:268`: `status=needs-review … last=window-closed`). The window reopened at 20:20. At 19:14 the dispatcher's own probe said `OPEN` and it logged `pick: every open row is parked: A142(needs-review), A143(needs-review) - waiting for a person`. A closed window is not a question for a person; the row sat idle with its resume handle for as long as nobody noticed.

The cause is one rule: `scripts/autonomy/state.py:79` skips every `needs-review` row at pick time (spec 169: a row that waits for a person must not hold the queue behind it). `window-closed` shares that status with real failures (`worker-rc=9`, `gate-skipped-disk`), so the pick cannot tell "waiting for a person" from "waiting for the clock".

## Verified Premises
- **CONFIRMED** (read 2026-10-25) `state.py` `pick` iterates the plan in order and skips a row whose status is `parked` or `needs-review`, printing nothing for them but a stderr note; for a dispatchable row it prints TSV whose LAST field is the row's `session`; `cycle.sh:106` reads that field as `RSID` and passes `--resume` (`cycle.sh:168`). `state.py` honours `PII_STATE` (the state JSON) and `PII_PLAN` (the plan TSV) from the environment, so a test needs no real queue. `set` writes `status`, `session`, `last` as given.
- **CONFIRMED** the window itself is already guarded elsewhere: `tick.sh` refuses to start a cycle while `window-closed-until` is fresh (30 min throttle) and the cycle probes the account first (`cycle.sh:54-60`, exit 3, nothing spent). `pick` therefore does not need to know the time.
- **MEASURE FIRST.** The exact JSON shape `state.py load()` reads and writes (read `load`/`save` before writing a fixture), and what an empty `session` looks like in a row that never had one.
- **NOT CLAIMED**: that a resumed worker succeeds. Only that the row is offered again, with its session, once the window guard lets a cycle start.

## Requirements
### Functional
- **R1. `pick` offers a window-parked row.** In `scripts/autonomy/state.py`, a row with `status == "needs-review"` AND `last == "window-closed"` AND a non-empty `session` is dispatchable exactly like a `pending` row (same plan-order position, same TSV, session as the last field). A `needs-review` row with any other `last`, a `window-closed` row with an EMPTY session, and every `parked` row stay skipped as today, with the same stderr note. Nothing else in `pick`, `get`, `set`, `list` changes, and `cycle.sh`, `tick.sh`, `window.sh` are not edited. Update the docstring line for `pick` to say it.
- **R2. Tests, with controls.** New `scripts/autonomy/tests/pick-resumes-window-closed.test.sh`, in the style of `scripts/autonomy/tests/cycle-disk-wiring.test.sh` (one `ok <name>` / `FAIL <name>` line per assertion, a final `<N> ok, <M> FAIL` line, exit 0 only when `M` = 0, scratch under `_tmp/` removed by its own `trap`). It runs the REAL `state.py` as `uv run --no-project python scripts/autonomy/state.py pick` with `PII_STATE` and `PII_PLAN` pointing at scratch files it writes. Cases: (a) THE BUG: row `X1` needs-review, `last=window-closed`, `session=S1`: `pick` prints a line whose first field is `X1` and whose last field is `S1`; (b) CONTROL: `X1` needs-review with `last=worker-rc=9` and a session: `pick` prints `NONE`; (c) CONTROL: `last=window-closed` with an EMPTY session: `NONE`; (d) CONTROL: `parked` with `last=window-closed` and a session: `NONE`; (e) order: a `pending` row listed BEFORE the window-parked row is picked first, and a window-parked row listed before a `pending` one is picked first; (f) after `state.py set X1 last=worker-ok` the same row is `NONE` again; (g) CONTROL: a `done` row is never picked; (h) a window-parked row that is first in the plan and a `needs-review` row of another kind second: only the first is offered.

### Non-Functional
- Every file under the 450-line gate. No new dependency. Python for `state.py` only; the test is shell.

## Exit Criteria
- [ ] `bash scripts/autonomy/tests/pick-resumes-window-closed.test.sh 2>&1 | tail -1 | grep -qE "^[0-9]+ ok, 0 FAIL"`
- [ ] `! bash scripts/autonomy/tests/pick-resumes-window-closed.test.sh 2>&1 | grep -q "^FAIL"`
- [ ] `uv run --no-project python -m py_compile scripts/autonomy/state.py`
- [ ] `grep -q "window-closed" scripts/autonomy/state.py`
- [ ] `git diff --quiet -- scripts/autonomy/cycle.sh scripts/autonomy/tick.sh scripts/autonomy/window.sh`
- [ ] `bash scripts/autonomy/tests/cycle-disk-wiring.test.sh 2>&1 | tail -1 | grep -qE "^[0-9]+ ok, 0 FAIL"`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

## Guardrails
- Do NOT edit `cycle.sh`, `tick.sh`, `window.sh` or `plan.tsv`. Do NOT touch the real `_tmp/autonomy/state.json`: every test sets `PII_STATE`. Do NOT make any other `needs-review` row dispatchable. Do NOT commit.

## Follow-ups (not this spec)
1. The banner line `waiting for a person` should name the rows that wait for the clock separately from the ones that wait for a person.
2. A `last=window-closed` row whose session no longer exists (a pruned session file) should fall back to a fresh dispatch instead of failing the resume.
