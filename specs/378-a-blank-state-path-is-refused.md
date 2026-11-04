# Spec: a blank state path is refused, not silently redirected

## Overview

A test fixture row appeared in the operator's live queue on 2026-11-04: row `X1`, session `S1`,
status `needs-review` — the exact JSON that `scripts/autonomy/tests/gate-refusal.test.sh` plants
in its scratch state file. It reached `_tmp/autonomy/state.json` because `state.py` reads its path
as:

```python
STATE = os.environ.get("PII_STATE") or os.path.join(ROOT, "_tmp", "autonomy", "state.json")
```

An **empty** `PII_STATE` is falsy, so `PII_STATE= ... state.py <anything>` falls back to the
operator's live file. A test that forgets the variable, or expands an unset scratch path into it,
therefore writes the operator's real queue — and does so silently, with exit status 0.

This is a data-integrity defect, not a cosmetic one: the queue is what the dispatcher acts on, so
a leaked fixture is a row the lane can claim, work and commit.

**Project**: /projects/pi-improved
**Phase**: autonomy harness

## Premises

- `PII_STATE` unset must keep today's behaviour: default to `_tmp/autonomy/state.json`. Every
  existing caller relies on it (`selftest.sh` 42/42, `gate-refusal.test.sh` 36/0).
- `PII_PLAN` and `PII_SPECS` follow the same `or default` shape in the same file; they carry the
  same hazard for `plan.tsv` and `specs/`.
- The operator's live state must not be rewritten by any check, ever. The repo already holds this
  rule for stores (`specs/134-no-fixed-path-store-in-any-test.md`); the autonomy state is a store.

## Requirements

- **R1** `PII_STATE` set to an empty or whitespace-only value is refused: `state.py` exits
  non-zero, prints a message naming the variable, and performs no read and no write.
- **R2** `PII_STATE` unset is unchanged: the default path is used (the control — the near-miss
  that must still take the normal path).
- **R3** The same treatment for `PII_PLAN` and `PII_SPECS`.
- **R4** The refusal is covered by an executed control in `scripts/autonomy/selftest.sh` (or a
  test beside it) that asserts the operator's state file is byte-identical after a blank-value run.

## Files

- `scripts/autonomy/state.py` — the resolution point and the refusal.
- `scripts/autonomy/selftest.sh` — the control that the live file is untouched.
- `scripts/autonomy/tests/gate-refusal.test.sh` — prove it still passes 36/0 with a set path.

## Exit Criteria

- [ ] `! PII_STATE= uv run --no-project python scripts/autonomy/state.py list >/dev/null 2>&1`
- [ ] `PII_STATE= uv run --no-project python scripts/autonomy/state.py list 2>&1 >/dev/null | grep -q 'is set but empty'`
- [ ] `uv run --no-project python scripts/autonomy/state.py list >/dev/null 2>&1`
- [ ] `bash scripts/autonomy/tests/gate-refusal.test.sh 2>&1 | grep -qE '^[0-9]+ ok, 0 FAIL'`
- [ ] `bash scripts/autonomy/tests/cycle-disk-wiring.test.sh 2>&1 | grep -qE '^(10|1[0-9]) ok, 0 FAIL'`

The byte-identical assertion lives in `selftest.sh` case 14 -- it runs too long for the
criteria runner's per-command budget, and the gate's autonomy step executes it: this row's
gate run reported `fresh-gate: PASS` at 20/20.

## Guardrails

- Do NOT change the default path, and do NOT make an unset variable an error: that would break
  every caller and is the near-miss this spec pins.
- Do NOT delete the `or` fallback for the unset case; only make the *set-but-blank* case refuse.
- Do NOT remove the `X1` fixture from the test — it is the evidence that the assertion works.
