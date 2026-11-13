# Spec: a shipped graph names a model role, not one vendor's model
#: depends=421 status=pending block=-

## Overview
Measured 2026-11-13 while setting up the live daemon: the planner node in `graphs/github-issue.toml` carries `model = "glm-5.3"`, a bare model id. The daemon
resolves `model = "@<role>"` through `[models.<role>]` in the operator's configuration (`crates/pii-server/src/daemon/model_roles.rs`, spec 362) and passes
anything else through as the agent's model — so this graph can only run on a machine that reaches a model called `glm-5.3`, and on this box the only model
available is the local one behind `[llama] host`.

A shipped graph is part of the repository, not of a machine: it must name the ROLE it needs and leave the model to the operator. The graph already does
exactly that with capabilities; this row does it with the model.

**Project**: <repo>
**Phase**: pii manages its specs through GitHub
**Depends on**: 421

## Requirements
- R1 The planner node in `graphs/github-issue.toml` says `model = "@plan"`: a role, not a model id. Nothing else in that node changes.
- R2 The operator's `[models.plan]` table decides the model and the provider. No file in the repository names a vendor model for this graph, and the graph
  still lints with no new finding.
- R3 The choice is pinned where the gate runs: `scripts/checks/comment-and-template-vocabulary.sh` (which already reads this graph) refuses a planner node
  whose model is not a role, and names the file. A vendor id cannot come back unnoticed.
- R4 The daemon's resolution is not re-implemented here: a role that names a model the provider cannot serve is the daemon's error to report, at start.
- R5 No other shipped graph changes, and no `[models]` table is added to the repository.

## Architecture
```
graphs/github-issue.toml                     [[node]] id="plan"  model = "@plan"
operator's ~/.config/pii/pii.toml           [models.plan]  model = "<local model>"  provider = "llamacpp-server"
daemon                                      resolves "@plan" -> (model, provider) and hands both to the agent
scripts/checks/comment-and-template-vocabulary.sh   refuses a planner node naming an id instead of a role
```
Key decision: a role is the seam between the repository and the machine, exactly as a capability is. Rejected: naming the local model in the graph (the same
defect one step later — the repository would depend on this box's llama.cpp). Rejected: a `[models]` table in the repository (graphs do not carry an
operator's inventory).

## TDD Contract
The pin is a check the gate already runs; the graph's own Rust test cannot host it, because `crates/pii-server/tests/github_issue_graph.rs` is at its
450-line cap (measured 2026-11-13: adding assertions there failed `lint-files` at 466 lines).
| Test Name | Input | Expected Output |
|---|---|---|
| `the_planner_node_names_a_model_role_not_a_model_id` | `graphs/github-issue.toml` | the plan node's `model` starts with `@`; the check fails naming the file when it does not |
| `CONTROL_a_vendor_id_in_a_scratch_copy_is_refused` | the same check over a scratch copy with `model = "glm-5.3"` | the check reports it |

Acceptance: both pass, and putting `model = "glm-5.3"` back into the shipped graph turns the first one red.

## Exit Criteria
### New in this row
Each is verified to FAIL before the change. A criterion that already passes is a Guardrail, never a criterion.
- [ ] `grep -q 'model = "@plan"' graphs/github-issue.toml && bash scripts/checks/comment-and-template-vocabulary.sh`

## Guardrails
- Keep green, unedited: `cargo test -q -p pii-server --test github_issue_graph` (10 assertions) and `bash scripts/checks/report-row-comments-and-closes.sh`.
- Do NOT grow `crates/pii-server/tests/github_issue_graph.rs`: it is at its 450-line cap.
- Do NOT add a `[models]` table to any file in the repository, and do NOT touch any other graph.
- Every dirty path must be listed under Files: list a file before you touch it.

## Files
- `graphs/github-issue.toml`
- `scripts/checks/comment-and-template-vocabulary.sh`
