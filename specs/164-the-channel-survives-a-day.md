# Spec: the byte budget proof, and the oversized response

## Overview
The channel died 3h29m after start because the byte budget was cumulative for the life of the process. The fix is in;
this pins it: 50,000 scripted polls in a row with no byte-budget refusal, and one response larger than the window still
refused.

**Project**: /projects/pi-improved

## Exit Criteria
- [ ] `cargo test -p pii-net --test budget_window`

## Guardrails
- Do NOT weaken an existing budget or control to make this pass; change the design instead.
- Do NOT commit; the loop's orchestrator commits.
