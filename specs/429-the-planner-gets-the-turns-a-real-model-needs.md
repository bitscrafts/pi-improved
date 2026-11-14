# Spec: the planner gets the turns a real model needs
#: depends=428 status=pending block=-

## Overview
Measured live on 2026-11-14, issue #29: the plan node failed after a 1.1 s model call and the graph posted its refusal.
The node ran with `max_turns = 1` while offering the model the `fs_read` tool, and its `err`, `budget` and `fail` ports
all lead to `refused`. A real model offered a tool uses it: turn 1 returns a tool call, there is no turn 2, the
`budget` port fires, and no spec is written.

Proven by the same run with `max_turns = 3` (live daemon, installed graph copy): issue #30 produced
`specs/2-spec-queue-rejects-specs-missing-an-exit-criteria-section-re.md`, a real `glm-5.3` spec with R1–R8 and a TDD
table. The fixture never exposed it because a scripted reply contains no tool call.

## Requirements
- R1 The plan node in `graphs/github-issue.toml` declares enough turns for a tool round trip: `max_turns = 3`.
- R2 A test pins the floor, so a later edit cannot silently return it to 1: the shipped graph's plan node has
  `max_turns >= 2`.
- R3 Nothing else about the node changes: same role, same prompt, same tools, same edges.
- R4 The graph still lints and its own tests still pass.

## Architecture
```
plan (agent, tools = ["fs_read"], model = "@plan")
  turn 1 -> tool call (fs_read)      [a real model uses the tool it is offered]
  turn 2 -> the spec document       [max_turns = 3 leaves room]
  turn 3 -> unused
budget/err/fail -> refused
```

## TDD Contract
| Test Name | Input | Expected Output |
|---|---|---|
| `the_planner_node_has_room_for_a_tool_round_trip` | the shipped graph's plan node | `max_turns >= 2` |
| `the_graph_lints_and_has_no_queue_or_coder_node` | the whole graph | unchanged: passes as before |

## Exit Criteria
### New in this row
Verified to FAIL before the change: the plan node says `max_turns = 1`.
- [ ] `grep -q 'max_turns = 3' graphs/github-issue.toml && cargo test -q -p pii-server --test github_issue_graph 2>&1 | grep -qE 'test result: ok\. 1[0-9] passed'`

## Guardrails
- Do NOT change the role, the prompt, the tools or the edges (R3).
- Do NOT raise other nodes' turns: only the planner needs the round trip.
- Do NOT weaken the existing graph tests.

## Files
- `graphs/github-issue.toml`
- `crates/pii-server/tests/github_issue_graph/main.rs`

## Notes
The live daemon's installed copy (`$STATE/graphs/github-issue.toml`) was used as the test bed and is now AHEAD of the
repository (`max_turns = 3`). This row is what closes that gap: after it lands, reinstall the graph from the repo so the
two are identical again.
