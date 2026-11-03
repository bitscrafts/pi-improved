# Spec: An unknown phase marker means dispatch, not verify

**Project**: /projects/pi-improved · **Measured**: 2026-11-03, row 353.

## Why this exists

I re-queued five rows by writing prose into the lane's `last` field:

```
state.py set 353 status=pending attempts=0 last="re-queued 2026-11-03: earlier work lost to the wrong-spec defect …"
```

`last` is not a note field. It is the **phase marker** the cycle reads to decide where a row is. Given prose it could
not recognise, the cycle logged

```
[21:32:20Z] phase marker says the work is already re-queued 2026-11-03: earlier work lost to the wrong-sp…
[21:32:20Z] verify: cargo test -q -p pii-server --test queue_requeue_after_restart …
[21:32:21Z] VERIFY FAILED: grep -q 'timer.wheel.error' crates/pii-observe/src/names.rs
OUTCOME=verify-failed ITEM=353
```

— it skipped the dispatch entirely and went straight to verification, one second later, on an empty tree. The row
was then marked `verify-failed` for work that had never been attempted. A person reading the state would conclude
the row had failed; nothing had run.

`state.py repair` exists for re-queueing a parked row, and its marker is prefixed `park-cause-repaired: ` precisely
so the cycle can recognise it. The generic `set` path has no such guard.

## Requirements

- **R1** An unrecognised phase marker is treated as **not dispatched**: the cycle dispatches and says
  "unknown marker '<x>', dispatching".
- **R2** The markers the cycle acts on are enumerated in one place (a constant), not matched by prose heuristics.
- **R3** `state.py set <id> last=<value>` refuses a value that is not a known marker, and names the alternative
  verbs (`repair` for a park whose cause is fixed, `annotate` for a fact, `--note` for prose).
- **R4** A prose note has somewhere to live that the phase logic never reads, and `state.py get` shows it separately.

## Files

- `scripts/autonomy/state.py`
- `scripts/autonomy/cycle.sh`
- `scripts/checks/tests/state-phase-marker.test.sh`

## Exit Criteria

- [ ] `bash scripts/checks/tests/state-phase-marker.test.sh 2>&1 | grep -q 'RESULT: 0 failed'`
- [ ] `grep -q 'PHASE_MARKERS' scripts/autonomy/state.py`
- [ ] `grep -q 'unknown marker' scripts/autonomy/cycle.sh`

## Guardrails

- Do NOT make the cycle re-dispatch a row whose marker says a worker is running.
- Do NOT accept prose in `last`; refuse it and point at the right verb.
- Test both directions: a known marker keeps its meaning, an unknown one dispatches.
