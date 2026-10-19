# Spec: a cancelled run does not fire

## Overview
MEASURED 2026-10-19 04:20Z: four runs cancelled with pii run cancel (three answered {"cancelled":true}) all posted anyway. The cancel set the status; the timer kept its own copy and fired it. Spec 196 addressed the arming side; this row proves the cancellation side in production terms.

**Project**: /projects/pi-improved

## Requirements
After pii run cancel <id>, the run's timer must not fire: a scratch daemon with a two-minute cron, a cancelled run, and ten minutes of observation showing zero executions and zero posts for that run id.

## Exit Criteria
- [ ] cargo test -p pii-server --test cancelled_run_disarms
- [ ] cargo test -p pii-graph --test resume_skips_completed

## Guardrails
- Do NOT weaken the gate, a budget or any existing control to make this pass.
- Do NOT test on the live daemon: its digest is armed for the morning occurrence.
- A cancelled run stays visible in run ls with its reason.
