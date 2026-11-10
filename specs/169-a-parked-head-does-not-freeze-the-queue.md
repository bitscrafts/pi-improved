# Spec: a parked head does not freeze the queue

## Overview
Measured 2026-10-17: A22 failed the gate and was set to `needs-review`. It is the QUEUE HEAD, and the tick refuses to
dispatch a needs-review item - so A27, A28 and everything behind them never ran, no worker was launched, no model was
called, and the account window sat frozen for 35+ minutes while every watcher reported "fine". One bad row at the front
costs a whole night of silence.

**Project**: <repo>

## Requirements
The tick's pick must SKIP any item whose state is parked/needs-review and dispatch the next dispatchable row, naming the
skipped one in its log line. A parked item stays parked until a person clears it; it must not hold the queue behind it.
When every row is parked, the tick says so once rather than staying silent.

## Exit Criteria
- [ ] `cargo test -p pii-graph --test pick_skips_parked` (a parked head, a pending second: the tick must pick the second)
- [ ] `grep -q skip scripts/autonomy/state.py`

## Guardrails
- Do NOT clear a parked item automatically; only skip it, and only for dispatch order.
- Do NOT commit; the loop's orchestrator reviews and commits.
