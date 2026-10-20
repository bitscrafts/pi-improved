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

## Closure

Closed 2026-10-20 by the orchestrator. Every criterion ran on the tree the worker left, and the work itself had
already landed: it is the bulk of `aa6c721` (the leftover commit of cycle `20261020T044202Z`), which took
`refused_criterion_fails_the_node.rs`, `run_error_keeps_the_node_reason.rs`,
`trace_prints_the_refusal_reason.rs`, `cmd_trace.rs`, `agent_calls.rs` and `coder.rs`.

- criteria 1-3: green at HEAD — `refused_criterion_fails_the_node` 2 passed / 0 failed,
  `run_error_keeps_the_node_reason` 2/0, `trace_prints_the_refusal_reason` 2/0 (2026-10-20T06:56Z).
- criterion 4 (the live one) is now a command, not prose: `bash scripts/orch-224-live-unknown-criterion.sh`
  builds a graph whose coder node carries `criteria = ["cargo test"]` and drives it through the production
  binary (`pii graph run`) — so the criteria table under test is the shipped one, not a fixture's. Measured
  output: `NodeFailed ... "criteria.unknown: 'cargo test' is not in the criteria table"` and the same string
  as the run's terminal `Failure`; `coder.no_call_events` appears nowhere. rc=0.

Why the item sat in `needs-review` with `last=gated` while nothing was staged: the plan row's commit-path
column separated its three directories with `|`, and `cycle.sh` splits that column on `;` (L357), so `git add`
was handed the single nonexistent path `crates/pii-server/src|crates/pii-graph/src|crates/pii-cli/src` and
exited 11 (`add-failed`) after a green verify and a green gate. The row was also carrying 1 of the 4 criteria.
Both are row defects, not worker defects; the row is corrected. The same `|` typo was found in the commit-path
column of the finished rows A85, A87, A90 and A91 — corrected there too, since the class, not the instance, is
what re-fails.

