# Spec: the scheduled run has a runtime

## Overview
MEASURED 2026-10-17 23:05Z on the DEPLOYED daemon: with a [[cron]] entry present, the daemon starts, loads its graphs, and then panics on the scheduled path:

    thread '<unnamed>' panicked at crates/pii-server/src/engine/runs.rs:71:5:
    there is no reactor running, must be called from the context of a Tokio 1.x runtime

exit status 134 with the restart counter at 5 and climbing: a crash loop. The cron was removed to stop it. runs stood at 71 from that loop. THIS IS WHY the deployed daemon had run nothing for fourteen hours: not only could it not find its graphs, the ONE path configured to run on a schedule had never been executed and has never worked.

**Project**: /projects/pi-improved

## Requirements
A run started by a [[cron]] timer must execute inside a Tokio runtime, or the daemon must refuse the cron at start naming the defect. A panic that crash-loops the supervisor is not an acceptable failure mode for a scheduled task.

## Exit Criteria
- [ ] cargo test -p pii-server --test cron_runtime
- [ ] grep -n Handle::current crates/pii-server/src/engine/runs.rs

## Guardrails
- Do NOT weaken the gate, a budget or any existing control to make this pass.
- The fix must be exercised by a test that starts a run FROM A TIMER NODE, not only by unit-constructing the engine.
