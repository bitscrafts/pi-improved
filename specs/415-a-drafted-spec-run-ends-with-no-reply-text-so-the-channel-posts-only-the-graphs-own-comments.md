# Spec: a drafted-spec run ends with no reply text, so the channel posts only the graph's own comments
#: depends=413 status=pending block=-

## Overview
Measured 2026-11-13, end to end on a real `ready` issue (`bitscrafts/pi-improved#16`), with the shipped `graphs/github-issue.toml`: the issue got THREE comments where the graph posts two. The third was
`{"bytes":149,"channel":"github","duplicate":false,"key":"done-16","to":"16","truncated":false}`: the `post` node's own result, on a public page.

Cause, read in `crates/pii-server/src/daemon/cron.rs` (`on_finished`): when a channel run succeeds, the daemon replies on that channel with the run's final output (`output.text`, or the whole output as JSON when
there is no `text`). The graph's exit node `done` is a `post`, whose output has no `text`, so the whole result object is sent as a reply. `graphs/discord-triage.toml` avoids this: its last node is a `transform` with
`value = { text = "", outcome = "triaged" }`, and `on_finished` sends nothing for an empty text (spec 128). The unit tests missed it because they start the graph with `run_graph` and never go through the channel's reply path.

**Project**: <repo>
**Phase**: pii manages its specs through GitHub
**Depends on**: 413

## Requirements
- R1 `graphs/github-issue.toml` ends in a `transform` node whose value is `{ text = "", outcome = "..." }`, as `discord-triage` does, and it is the graph's `exit`. Both ways of ending (`done` after a draft, `refused` after a failure) lead to it.
- R2 The two comments the graph posts are unchanged: their texts, their keys (`ack-${input.chat}`, `done-${input.chat}`, `refused-${input.chat}`) and their order.
- R3 A run's final output has an empty `text` on the drafted path AND on the refused path, so `on_finished` sends nothing further.
- R4 The graph still lints with no error, and no longer warns that `done` or `refused` has an `ok` port with no outgoing edge.
- R5 The end node holds no capability and no issue text: its value is fixed.

## Architecture
```
ack -> plan -> write -> done ----\
        |        |               +--> end   (transform: text = "")  == exit
        +--err---+--> refused ---/
```
Key decision: the fix is in the graph, by the pattern the shipped chat graph already uses. Rejected: teaching `on_finished` to skip a `post` node's result (a second rule beside the one that already exists, and it would change every graph).

## TDD Contract
Added to `crates/pii-server/tests/github_issue_graph.rs` (414 lines; the two tests must keep it at 450 or fewer, otherwise make the helpers a folder module in the same row).
| Test Name | Input | Expected Output |
|---|---|---|
| `a_drafted_run_ends_with_an_empty_reply_text` | a delivered issue, planner returns a draft | the run's final output has `text` equal to `""` |
| `CONTROL_a_refused_run_also_ends_with_an_empty_reply_text` | a delivered issue, planner fails | the run's final output has `text` equal to `""`, and the fixed refusal was still posted once |

Acceptance: the existing 8 still pass unedited; the two new pass. Each of these mutations makes at least one fail (the mutation script is the orchestrator's): the end node removed (`done` is the exit again); the end node's `text` set to a non-empty string; the `refused` arm not wired to the end node.

## Exit Criteria
### New in this row
Each is verified to FAIL before the change. A criterion that already passes is a Guardrail, never a criterion.
- [ ] `cargo test -q -p pii-server --test github_issue_graph 2>&1 | grep -qE 'test result: ok. (9|[1-9][0-9]) passed'`

## Guardrails
- Keep green, unedited: the 8 tests already in `github_issue_graph.rs`, and `cargo test -q -p pii-server --test plan_writes_a_spec`.
- Do NOT change a comment's text or key, and do NOT add a node that holds a capability.
- Do NOT edit any Rust source under `crates/*/src/`: the fix is the graph and its tests.
- Every dirty path must be listed under Files: list a file before you touch it.

## Files
- `graphs/github-issue.toml`
- `crates/pii-server/tests/github_issue_graph.rs`
