# Spec: NodeProgress and a mandatory timeout for process nodes

## Overview
A node that can block on a process must say it is alive. Give PROC node kinds a NodeProgress event carrying elapsed and
child cpu, and make timeout_ms mandatory on them, so a node that wedges is distinguishable from one that works.

**Project**: /projects/pi-improved

## Exit Criteria
- [ ] `cargo test -p pii-graph --test progress_contract`

## Guardrails
- Do NOT weaken an existing budget or control to make this pass; change the design instead.
- Do NOT commit; the loop's orchestrator commits.
