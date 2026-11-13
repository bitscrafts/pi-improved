# Spec: an agent node resolves the model role it names, or refuses it by name
#: depends=422 status=pending block=-

## Overview
Spec 362 gave a node `model = "@<role>"`, filled from the operator's `[models.<role>]`, so a shipped graph names a role
and the operator picks the model. Measured 2026-11-13 on the live daemon: the `github-issue` graph's planner node
(`kind = "agent"`, `model = "@plan"`) was handed to the model layer **unresolved**. The log said
`model.provider.in_force {model:"@plan", provider:"openai"}`, the call went to the local llama.cpp host, and after 395 s
it returned empty content, so every issue ended in the refusal branch.

The cause is in the code: `resolve_role` is called only from the coder node kinds
(`node_kinds/nodes/coder.rs`, `node_kinds/nodes/coder/graph_node.rs`). The agent node kind resolves its model through
its own `resolve_model` (spec 28: route step, then the `model` key, then the provider default) and never consults the
role table. So `@plan` is not a role here — it is a model *id*, and an unknown id falls through to a provider default
instead of being refused, which spec 362 R3 forbids by name.

## Requirements
- R1 An `agent` node whose `model` starts with `@` is resolved through the daemon's model-role table **before** any
  provider is built and before any model call, exactly as the coder node resolves it: the call carries the role's
  `model` and `provider`.
- R2 A role the operator's config does not define is **refused by name** before any model call, and the refusal names
  both the node id and the role, listing the configured roles. It is never defaulted to another provider (spec 362 R3).
- R3 An explicit model id (one that does not start with `@`) is passed through unchanged and keeps today's behaviour,
  including the provider default.
- R4 The resolution happens once per node start, not per turn.
- R5 No change to the coder node's resolution, to route steps, or to `[[model_tier]]` selection.
- R6 The role table reaches the agent node the same way it reaches the coder node: one field on the node's build
  inputs, so there is a single source of truth for it.
- R7 `crates/pii-server/src/lib.rs` may not grow past its recorded 669 lines: it is baselined and the baseline only
  shrinks. If the plumbing would add lines there, take them from an existing block instead.

## Architecture
```
config: [models.plan] model/provider
            |
            v
  role table (BTreeMap<String, ModelRole>)
            |
   +--------+---------+
   |                  |
 coder node        agent node   <-- today this path does not exist
   |                  |
 resolve_role()     resolve_role()  (new, before the provider is built)
   |                  |
   v                  v
  call with the role's model + provider
```

## TDD Contract
| Test Name | Input | Expected Output |
|---|---|---|
| `an_agent_node_resolves_the_role_it_names` | a graph whose agent node says `model = "@plan"`, a role table with `plan = (fast-model, fake-provider)` | the provider receives `fast-model`, and the run succeeds |
| `CONTROL_an_unknown_role_is_refused_by_name` | the same node, an empty role table | the run fails before any model call; the message names the node id and `@plan` |
| `CONTROL_an_explicit_model_id_is_passed_through` | `model = "fast-model"`, a role table that defines `plan` | the provider receives `fast-model`, unchanged |
| `CONTROL_a_failing_role_call_names_the_role` | the role's provider returns an error | the failure names the role, so an operator knows which entry to fix |

## Exit Criteria
### New in this row
Verified to FAIL before the change: no such test file exists, and the agent node never consults the role table.
- [ ] `cargo test -q -p pii-server --test agent_node_model_roles 2>&1 | grep -qE 'test result: ok. [4-9] passed'`

## Guardrails
- Do NOT resolve a role inside the per-turn loop: build the provider once (R4).
- Do NOT let an unknown role fall through to a provider default (R2) — that is the defect this spec exists for.
- Do NOT change the coder node, route steps, or tiers (R5).
- Do NOT edit existing tests to make this pass.
- Do NOT grow `crates/pii-server/src/lib.rs` (R7).

## Files
- `crates/pii-server/src/engine/registry.rs`
- `crates/pii-server/src/node_kinds/agent/mod.rs`
- `crates/pii-server/src/node_kinds/agent/tests.rs`
- `crates/pii-server/tests/agent_node_model_roles.rs`
- `crates/pii-server/tests/cancellation_brake.rs`
- `crates/pii-server/tests/fail_port.rs`
- `crates/pii-server/tests/failover_graph.rs`
- `crates/pii-server/tests/github_issue_graph.rs`
- `crates/pii-server/tests/github_issue_graph/harness.rs`
- `crates/pii-server/tests/github_issue_graph/main.rs`
- `crates/pii-server/tests/model_routing.rs`
- `crates/pii-server/tests/model_tiers.rs`
- `crates/pii-server/tests/review_graph/recording.rs`
