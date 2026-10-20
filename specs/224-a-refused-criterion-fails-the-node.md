# Spec: a refused criterion fails its node, and the run keeps THAT error

## Overview
Measured 2026-10-20 on lnx with the daemon log open (`pii loop events --log`), the coder node's real reason finally appeared:

    {"kind":"criteria.unknown","level":"warn","subsystem":"coder",
     "fields":{"agent":"pi","criterion":"cargo test","model":"glm-5.3-flash:cloud"}}

So the node refused because `cargo test` is not a NAMED criterion (spec 215's table). Two defects hid it:

1. the run reported `coder.no_call_events` instead of the node's own `criteria.unknown`, because a node that refuses its
   criterion still reports SUCCESS to the run and the 221 check then fires. The operator sees the symptom, never the cause.
2. `pii run trace` printed `fail=-` for that node, so the reason existed only in the daemon log.

## Requirements
1. An unknown criterion is a NODE failure: `Outcome::Err` with `criteria.unknown{agent, criterion, model}`, and the run's
   terminal error is that message verbatim — never `coder.no_call_events`.
2. `pii run trace` prints that reason on the node's line in `fail=<reason>`, truncated to 80 chars.
3. `pii run explain` includes the same sentence for the failing node.
4. The 221 check (`no_call_events`) applies ONLY to a node whose own outcome is success.

## Exit Criteria
- [ ] `cargo test -p pii-server --test refused_criterion_fails_the_node`
- [ ] `cargo test -p pii-graph --test run_error_keeps_the_node_reason`
- [ ] `cargo test -p pii-cli --test trace_prints_the_refusal_reason`
- [ ] on lnx: a coder node with `criteria = ["cargo test"]` produces a run whose error string contains `criteria.unknown`

## Guardrails
- Do NOT widen the criteria table to accept a shell string to make a demo pass; add a NAMED entry instead.
- Do NOT let a run-level check overwrite a node-level error.
