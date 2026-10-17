# Spec: the operator acts are recorded

## Overview
Doc 159 measured operator distance and stated its own weakness in plain words: "the store does not yet record operator
actions. H_pre and A read the optional operator_acts array. No incident carries them, so H_pre=false and A=0 here mean
'none recorded', not 'measured as none happened'." Until the loop writes operator_acts, only L is a real measurement.

**Project**: /projects/pi-improved

## Requirements
- Every place a person acts on the loop (clearing a park, approving or refusing a card, pausing or resuming, editing a
  queue row) writes an operator_acts record with the act, the item, the timestamp and the actor.
- `pii incidents distance --since <date>` then reports H_pre and A from recorded acts, and still NAMES any missing
  timestamp rather than imputing it.

## Exit Criteria
- [ ] `cargo test -p pii-learn --test incidents_runtime` (a control: an act recorded, and an incident with none still prints "none recorded")
- [ ] `pii incidents distance --since 2026-10-17` prints H_pre and A with a denominator, not a caveat-only line

## Guardrails
- Do NOT impute a missing act; name it and count it.
- Do NOT commit; the loop's orchestrator reviews and commits.
