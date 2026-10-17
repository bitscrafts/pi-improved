# Spec: the in-flight barrier, time-scoped

## Overview
D3 asks that a node's start be durable while it runs. A flush per node start costs ~100 write transactions per run; the
write-amplification budget allows ~2 (measured: "a run costs 304/100 commits; expected ~2"). Flush at most once per N
seconds instead, so a fast run pays one transaction and a long node's start lands within N seconds.

**Project**: /projects/pi-improved

## Exit Criteria
- [ ] `cargo test -p pii-graph --test write_amp && cargo test -p pii-server --test boss_watch`

## Guardrails
- Do NOT weaken an existing budget or control to make this pass; change the design instead.
- Do NOT commit; the loop's orchestrator commits.
