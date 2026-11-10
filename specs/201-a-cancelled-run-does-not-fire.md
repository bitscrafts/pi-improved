# Spec: a cancelled run does not fire

## Overview
Written 2026-10-19 to close a bookkeeping hole: row A65 named this file and it did not exist. The substance of this work shipped as spec 196 (5a6445e) and was re-proven as spec 198; the four posts at 09:00:01Z came from runs whose records were cancelled while their timer-wheel entries stayed armed.

**Project**: <repo>

## Requirements
`pii run cancel <id>` must disarm the run's timer as well as closing its record. A cancelled run never executes, and the cancellation is visible with its reason.

## Exit Criteria
- [ ] cargo test -p pii-server --test cancelled_run_disarms

## Guardrails
- Do NOT weaken the gate, a budget or any existing control to make this pass.
- Do NOT delete the run record: cancellation is a status with a reason.
