# Spec 249: the queue's first spec implemented by pii (a smoke test of the lane itself)

## Overview
**Project**: <repo>
**Phase**: production — the queue's worker is pii (user directive 2026-10-24)

This spec is deliberately tiny. It exists to prove the new lane (`scripts/run-batchC-pii.sh`: a coder-node graph run by the pii daemon inside the devbox
container) from dispatch to a verified result, on a change that cannot hurt anything.

## Requirements
### Functional
- **R1.** Create the file `_docs/05-operations/pii-lane-smoke.txt` containing exactly one line: `implemented by pii`.

## Exit Criteria
- [ ] `test "$(cat _docs/05-operations/pii-lane-smoke.txt)" = "implemented by pii"`

## Guardrails
- Do NOT create or change any other file.
- Do NOT commit.
