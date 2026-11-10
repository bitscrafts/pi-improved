# Spec: a cancelled run does not fire, proven on the deployed binary

## Overview
Four runs were cancelled (three answered cancelled:true) and all four posted at 09:00:01Z. Spec 196 fixed the timer-wheel leak and is now DEPLOYED (420fe8a7e946). This row is the PROOF in production terms, not the fix.

**Project**: <repo>

## Requirements
On a SCRATCH daemon with a two-minute cron: start a run, cancel it, observe ten minutes. Zero executions and zero posts for that run id, and the cancelled run remains visible in run ls with its reason.

## Exit Criteria
- [ ] cargo test -p pii-server --test cancelled_run_disarms

## Guardrails
- Do NOT weaken the gate, a budget or any existing control to make this pass.
- Do NOT run this on the live daemon: its digest is armed for the daily occurrence.
