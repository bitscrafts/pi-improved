# Spec: a called graph's prompt placeholders follow the same renaming as its expressions
#: depends=255 status=pending block=-

## Overview
Row 403 built a fan-out graph (`graphs/review.toml`) that calls a one-reviewer graph (`graphs/review-one.toml`) once per rubric slot. The
one-reviewer graph runs alone and the worker's own tests pass for it. The fan-out is refused at load, so 3 of the 19 tests in
`crates/pii-server/tests/review_graph.rs` fail (measured 2026-11-10: `review_with_all_slots_passing_ends_ok`,
`review_with_one_failing_slot_names_that_rubric_only`, `CONTROL_a_skipped_slot_is_not_a_failure`), all with:
`V12: node 'slot_1__reviewer' placeholder '{{nodes.rubric.stdout}}' names a path the graph cannot supply`.

Cause, read in the code: a `call` node inlines its sub-graph and renames every node `<call>__<id>`. `rewrite` in
`crates/pii-graph/src/fragment/call_rewrite.rs` (line 22) renames a reference only in a string that starts with `$.`, so
`$.nodes.rubric` becomes `$.nodes.slot_1__rubric`. A `prompt` is a template whose references are `{{path}}` placeholders (`template.rs`), checked at
load by V12 (`validate.rs`, against the renamed node ids) and rendered with the same evaluator. Nothing renames them, so the placeholder keeps
naming `rubric`, which no longer exists. Any called graph that has an agent node with a `{{nodes.X}}` placeholder is affected, not only this one.

**Project**: <repo>
**Phase**: graph composition (follows 255, which created `call`)
**Depends on**: 255

## Requirements
- R1 A `{{path}}` placeholder in an inlined node's `prompt` is renamed by the same rule as the `$.path` expression: `{{nodes.m.rest}}` becomes
  `{{nodes.<call>__m.rest}}` when `m` is a node of the sub-graph; `{{input.key...}}` follows the call's input mapping exactly as `$.input.key` does;
  `{{payload.arm...}}` follows the join-arm rule. There is ONE rule: the placeholder path is run through the existing `rewrite_expr`, not a second copy of it.
- R2 Only the `prompt` field of an inlined node is rewritten this way. Every other field keeps today's behaviour: a string that does not start with `$.`
  is left alone, including a `system`, `value` or `brief` that happens to contain `{{...}}`.
- R3 A placeholder whose rewrite is the same text it started with is left byte for byte (spacing inside the braces included). Text outside placeholders is never touched.
- R4 A placeholder that names no node of the sub-graph is not renamed, so V12 still refuses it at load, naming the placeholder.
- R5 `graphs/review.toml` (row 403) loads after this change. That is verified by row 403's own criterion, not by this spec.

## Architecture
```
splice (call.rs)  --for each field-->  rewrite_field(key, value, refs)      (call_rewrite.rs)
                                         key == "prompt" and a string  -> rewrite_prompt(text, refs)
                                         anything else                 -> rewrite(value, refs)   (unchanged)
rewrite_prompt: for each {{path}}: next = rewrite_expr("$." + path); keep the original text if next == original
```
Key decisions:
- One renaming rule (`rewrite_expr`), applied to a different carrier. Rejected: a second regex-based renamer in the validator; the validator would then
  accept what the runner cannot render.
- Restricted to `prompt`. Rejected: rewriting every string that contains `{{`; a `system` literal or a `value` is data, and renaming inside it changes what the node says.
- `call.rs` is already over the line limit (466 lines against 450, baselined). The change must not add a line to it: the call site swaps `rewrite(v, &refs)` for
  `rewrite_field(k, v, &refs)` in place, and the new code lives in `call_rewrite.rs` (67 lines).

## TDD Contract
| Test Name | Input | Expected Output |
|---|---|---|
| `a_prompt_placeholder_naming_a_subgraph_node_is_renamed_with_it` | sub-graph: transform `a`, agent `b` with prompt `Rubric: {{nodes.a}}`; host calls it as `docs` | the woven node `docs__b` has prompt `Rubric: {{nodes.docs__a}}` and the graph loads |
| `CONTROL_a_placeholder_naming_no_subgraph_node_is_still_refused` | the same prompt with `{{nodes.zzz}}` | load fails with V12 naming `{{nodes.zzz}}`; the prompt text is unchanged |
| `a_prompt_input_placeholder_follows_the_call_mapping` | prompt `T: {{input.title}}`, call maps `title` | the prompt names the call's mapped input node, the same node `$.input.title` reaches, and the graph loads |
| `CONTROL_a_system_and_a_value_containing_braces_are_not_rewritten` | the agent node's `system` is `see {{nodes.a}}`; a transform's `value` is `{{nodes.a}}` | both strings unchanged after weaving |
| `CONTROL_an_unrelated_placeholder_keeps_its_spacing` | prompt `{{ payload.text }} and {{nodes.a}}` where `payload.text` is not a join arm | `{{ payload.text }}` is byte-identical; `{{nodes.a}}` is renamed |
| `a_graph_without_a_prompt_placeholder_weaves_as_before` | the `SUB` and `HOST` of spec 255's `call_inlines_a_graph.rs` | identical woven node ids and fields |

Acceptance test: `a_prompt_placeholder_naming_a_subgraph_node_is_renamed_with_it`. The dangerous requirement is R2: a plausible implementation renames every
`{{...}}` in every field, passes the positive row, and fails only the `CONTROL_a_system_and_a_value_containing_braces_are_not_rewritten` row. A second
plausible implementation renames inside `prompt` but with its own pattern instead of `rewrite_expr`; the `input` row catches it, because a hand-written rule
renames `nodes` and forgets the call's input mapping.

## Exit Criteria
### New in this row
Each is verified to FAIL before the change. A criterion that already passes is a Guardrail, never a criterion.
Not a criterion: that the test file exists. The lane writes it in its own first step, so a `test -f` on it passes before the
work and the RED step refuses the row (measured 2026-11-10). The criterion below needs at least one passing test in that file.
- [ ] `cargo test -q -p pii-graph --test call_rewrites_prompt_placeholders 2>&1 | grep -qE 'test result: ok. [1-9][0-9]* passed'`
- [ ] `grep -q 'fn rewrite_prompt' crates/pii-graph/src/fragment/call_rewrite.rs`

## Guardrails
- Keep green (they pass today, so they are not criteria): `cargo test -q -p pii-graph --test call_inlines_a_graph`, `cargo test -q -p pii-graph --test call_needs` and `cargo test -q -p pii-graph --test call_outputs_and_cycles`.
- `crates/pii-graph/src/fragment/call.rs` must not get longer: `test "$(wc -l < crates/pii-graph/src/fragment/call.rs)" -le 466`.
- Do NOT add a second renaming rule: `grep -c 'fn rewrite_expr' crates/pii-graph/src/fragment/call_rewrite.rs` stays 1.
- Do NOT touch `validate.rs` or `template.rs`: the validator is right to refuse a placeholder it cannot resolve; the inliner must supply a name it can.
- Do NOT add a node kind, an expression function or a dependency.
- Do NOT edit `graphs/review.toml`, `graphs/review-one.toml` or `crates/pii-server/tests/review_graph.rs`: they belong to row 403 and are uncommitted work in the tree.
- Every dirty path must be listed under Files: list a file before you touch it.

## Files
- `crates/pii-graph/src/fragment/call_rewrite.rs`
- `crates/pii-graph/src/fragment/call.rs`
- `crates/pii-graph/tests/call_rewrites_prompt_placeholders.rs`
