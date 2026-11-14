# Spec: a refused draft records why the planner failed
#: depends=424 status=pending block=-

## Overview
Measured 2026-11-14 on the live daemon, issue #29: the planner role resolved, its provider was built through the
daemon's transport, and a real `glm-5.3` call answered in 1.1 s. The graph then took its refusal branch, and **nothing
anywhere says why**:

- the issue's comment is a fixed sentence (`pii: refused — a spec could not be drafted from this issue.`), because
  spec 421 deliberately never echoes internal text onto a public page;
- `pii run get` says `status: "succeeded"` — a modelled refusal is not a failure;
- `pii run explain` repeats that and adds nothing;
- the daemon log has no line for the failing node.

The run's trace shows `ack → plan → refused`: the `plan` node (kind `agent`, 1 turn) failed after its model call. An
operator cannot see the reason, so a channel that fails every issue looks identical to one that is working. This is the
defect: the refusal path is correct (nothing public), but the reason must survive somewhere an operator can read.

## Requirements
- R1 When a graph node ends on a branch that leads to a refusal, the daemon writes ONE log event naming the run, the
  node id and the node's error text, at `warn` level. The event kind is stable and greppable (e.g. `graph.node.failed`).
- R2 The event carries the node's own error text verbatim, because that text is the diagnosis. It is a log line, never a
  comment, so the public page stays free of it (spec 421 R3 unchanged).
- R3 The run's stored summary keeps `status: "succeeded"` for a modelled outcome — a refusal is not a run failure — but
  `pii run explain` prints the reason of the last failed node when one exists.
- R4 No comment, issue or PR text changes: the fixed sentences stay exactly as they are.
- R5 A node that succeeds writes no such event, so a healthy run's log stays quiet (a control).

## Architecture
```
plan node -> Err(reason)
     |            \
     |             +--> branch `err` -> refused (fixed sentence) -> end        [public: unchanged]
     |
     +--> graph.node.failed { run, node: "plan", error: reason }                [operator: new, R1]
```

## TDD Contract
| Test Name | Input | Expected Output |
|---|---|---|
| `a_failed_node_names_itself_in_the_log` | a two-node graph whose first node fails | one `graph.node.failed` event with the run, the node id and the error text |
| `CONTROL_a_healthy_run_writes_no_failure_event` | a graph whose nodes all succeed | no `graph.node.failed` event |
| `explain_prints_the_last_failed_node` | the failing run above | `run explain` output contains the node id and its reason |

## Exit Criteria
### New in this row
Verified to FAIL before the change: no `graph.node.failed` event exists anywhere in the tree or the log.
- [ ] `cargo test -q -p pii-server --test a_refused_reason_is_recorded 2>&1 | grep -qE 'test result: ok\. [3-9] passed'`

## Guardrails
- Do NOT put the reason into a comment, an issue body or a PR body (R4).
- Do NOT change a run's status to failed for a modelled refusal (R3).
- Do NOT log on success (R5).
- Do NOT grow a baselined file; extract instead.
- Do NOT weaken spec 421's fixed sentences or its check.

## Files
- `crates/pii-server/src/engine/runs.rs`
- `crates/pii-server/src/node_kinds/agent/mod.rs`
- `crates/pii-server/tests/a_refused_reason_is_recorded.rs`

## Notes
The node that fails is `kind = "agent"` with `max_turns = 1` and `tools = ["fs_read"]`. Its error is produced after the
model call returns, so the likely causes are the turn cap or the reply's shape; the event this row adds is what makes
that answerable in one log line instead of a guess.
