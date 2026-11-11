# Spec: a refusal's reason names what was refused
#: depends=409 status=pending block=-

## Overview
Row 403's reviewer graph refuses a review when one rubric cannot be read, and the test `review_with_one_failing_slot_names_that_rubric_only` requires the refusal to NAME that rubric
(`omega-missing`) and no other. It is the last of the 19 `review_graph` tests still red (measured 2026-11-11, after rows 404 and 409 landed: 18 pass, 1 fails with
`run.refused{hook=review, reason=a requested rubric did not pass, or rubric_1 was not given; ...}`).

The graph cannot do it today. A `refuse` node's `reason` is a fixed string in the TOML; `RefuseNode` (`crates/pii-graph/src/nodes.rs`) appends only a `reason` that the ARRIVING verdict already carries.
A `transform` computes one value, never an object, so the graph cannot build a reason either. A prompt can name a value (`{{input.rubric}}`, rendered by `template::render`); a refusal cannot.

This is the same gap row 404 closed for prompts, one node kind over: a node whose text mentions something that exists only at run time.

**Project**: <repo>
**Phase**: graph composition (follows 409)
**Depends on**: 409

## Requirements
- R1 A `refuse` node's `reason` may hold `{{path}}` placeholders, `path` being what a `prompt` placeholder may name (`input.x`, `nodes.n.field`, `payload...`). At run time each is replaced by the value it resolves to in the node's own
  eval context, with the SAME renderer a prompt uses (`template::render`: a string is inserted as is, anything else as its JSON text). One renderer, not a second one.
- R2 A refusal always refuses. A placeholder that resolves to nothing, or a reason with an unclosed `{{`, does NOT turn the refusal into a different error: the reason is used as written, placeholders and all.
- R3 A reason with no `{{` is byte-for-byte what it is today, in every case (`returns` set or not, an arriving verdict that carries a reason or not).
- R4 A called graph's refuse node is renamed by the inliner like a prompt: a `{{nodes.X}}` or `{{input.k}}` in its `reason` follows the same renaming as `$.nodes.X` / `$.input.k` (the 404 rule, `rewrite_prompt`, now also applied to the key `reason`).
  Only `prompt` and `reason` take it; `system`, `value`, `brief` stay data.
- R5 The text appended from an arriving verdict (`why`) stays appended after the rendered reason, as today.
- R6 `crates/pii-graph/src/nodes.rs` does not get longer: `RefuseNode` and its `GraphNode` impl MOVE to `crates/pii-graph/src/nodes/refuse.rs` and are re-exported from `nodes`, so the type's path does not change for any caller.

## Architecture
```
RefuseNode::run(ctx, input)
    arm / why                         (unchanged)
    reason = render_or_literal(self.reason, eval ctx of this node)   <- R1, R2
    Fail  { run.refused{hook, reason: "<reason>[: why]"} }           (unchanged shape)
    Next(ok, {verdict:"refused", port, reason: "port: <reason>[: why]", criterion})   (unchanged shape)
```
Key decisions:
- Render at RUN time, against the node's context, not at load. The value (`input.rubric`) exists only when a run supplies it, and a loader cannot know it.
- Fall back to the literal text, not an error (R2). A refusal that fails to render would hide the very failure it reports.
- The inliner rule is row 404's, widened by one key. No new renaming code.

## TDD Contract
New test file `crates/pii-graph/tests/refuse_reason_placeholders.rs`.

| Test Name | Input | Expected Output |
|---|---|---|
| `a_reason_placeholder_is_rendered_from_the_run_input` | refuse node, reason `rubric {{input.rubric}} did not pass`, run input `rubric = "omega"` | the failure text holds `rubric omega did not pass` |
| `a_reason_placeholder_reads_the_arriving_payload` | reason `bad: {{payload.name}}` | the payload's `name` appears |
| `CONTROL_an_unresolvable_placeholder_keeps_the_reason_as_written` | reason `x {{input.nope}} y` | the failure text holds `x {{input.nope}} y`, still a refusal |
| `CONTROL_an_unclosed_placeholder_keeps_the_reason_as_written` | reason `x {{input.rubric` | refusal with that text |
| `CONTROL_a_reason_without_braces_is_unchanged` | plain reason, with and without `returns` | byte-identical to the old output, both shapes |
| `the_arriving_reason_is_still_appended` | verdict carrying `reason = "boom"` | `<rendered>: boom` |
| `a_called_graphs_reason_placeholder_follows_the_renaming` | sub-graph refuse reason `{{nodes.m.x}}`, called as `c` | woven reason reads `{{nodes.c__m.x}}` |
| `CONTROL_a_system_field_holding_braces_is_not_rewritten_by_the_reason_rule` | node with `system = "{{nodes.m.x}}"` and a refuse beside it | `system` unchanged |

Acceptance test: `a_reason_placeholder_is_rendered_from_the_run_input`. The dangerous requirement is R2: a plausible implementation calls `render(...)?` and lets a missing path surface as an error, passes the
positive row, and fails only `CONTROL_an_unresolvable_placeholder_keeps_the_reason_as_written`.

## Exit Criteria
### New in this row
Each is verified to FAIL before the change. A criterion that already passes is a Guardrail, never a criterion.
- [ ] `cargo test -q -p pii-graph --test refuse_reason_placeholders 2>&1 | grep -qE 'test result: ok. [1-9][0-9]* passed'`
- [ ] `test -f crates/pii-graph/src/nodes/refuse.rs && grep -q 'render' crates/pii-graph/src/nodes/refuse.rs`

## Guardrails
- Keep green (they pass today): `cargo test -q -p pii-graph --test call_inlines_a_graph`, `cargo test -q -p pii-graph --test call_rewrites_prompt_placeholders`, `cargo test -q -p pii-graph --test call_rewrites_compound_expressions`,
  `cargo test -q -p pii-graph --test call_needs`, `cargo test -q -p pii-graph --test call_needs_reads_every_reference`.
- `crates/pii-graph/src/nodes.rs` must not get longer than today: `test "$(wc -l < crates/pii-graph/src/nodes.rs)" -le 583`.
- `grep -c 'fn render' crates/pii-graph/src/template.rs` stays 1: no second renderer.
- Do NOT touch `expr.rs`, `template.rs`, `validate.rs`, or the expression language.
- Do NOT edit `graphs/review.toml`, `graphs/review-one.toml` or `crates/pii-server/tests/review_graph.rs`: they are row 403's parked work.
- Do NOT add a node kind, a dependency, or a new field: `reason` already exists.
- Every dirty path must be listed under Files: list a file before you touch it, and a test that asserts on the old refuse text goes here too.

## Files
- `crates/pii-graph/src/nodes.rs`
- `crates/pii-graph/src/nodes/refuse.rs`
- `crates/pii-graph/src/fragment/call_rewrite.rs`
- `crates/pii-graph/tests/refuse_reason_placeholders.rs`
