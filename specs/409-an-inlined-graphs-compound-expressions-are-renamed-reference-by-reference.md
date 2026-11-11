# Spec: an inlined graph's compound expressions are renamed reference by reference
#: depends=404 status=pending block=-

## Overview
Row 404 made a called graph's prompt placeholders follow the renaming of its `$.` expressions. Row 403's fan-out (`graphs/review.toml` calling `graphs/review-one.toml`)
then loaded, and its three fan-out tests still fail (measured 2026-11-11, in a scratch worktree holding the parked 403 work): `review_with_all_slots_passing_ends_ok`,
`review_with_one_failing_slot_names_that_rubric_only` and `CONTROL_a_skipped_slot_is_not_a_failure`.

Cause, read from the failing run's journal: the `path` node of `review-one` is
`expr = '$.nodes.root + "/rubrics/" + $.input.rubric + ".md"'`. After inlining, node `root` is named `slot_1__root`, but `path` still reads `$.nodes.root`, finishes on port `err`
with `invalid: '+' needs two integers or two strings, got nothing and a string`, every slot's rubric read fails, every slot reports `failed`, and the run is refused.

`rewrite_expr` in `crates/pii-graph/src/fragment/call_rewrite.rs` splits the WHOLE string on `.` and matches only the first reference (`segs[0]`/`segs[1]`). For a single reference such as
`$.nodes.root` that is right. For an expression with several references, or any operator, `segs` is nonsense (`["nodes", "root + \"/rubrics/\" + $", "input", ...]`), no arm matches,
and the text is returned unchanged. The expression language (`expr.rs`) has had `+`, comparisons, `length`, `lower`, `trim`, `starts_with`, `ends_with` and `contains` since spec 383/392, so any
called graph that computes anything from a node or an input is affected, not only this one.

**Project**: <repo>
**Phase**: graph composition (follows 404)
**Depends on**: 404

## Requirements
- R1 Every `$.path` reference inside an expression string is renamed by the existing single-reference rule, wherever it stands in the expression: before or after an operator, as an argument
  of `length`/`lower`/`trim`/`starts_with`/`ends_with`/`contains`, inside parentheses, and on both sides of a comparison.
- R2 The text between references is not touched, byte for byte: operators, spacing, string literals and their escapes. A `$.` inside a double-quoted string literal is NOT a reference and is
  not renamed (`"$.nodes.root"` stays as written).
- R3 A reference that names nothing of the sub-graph (a host-run `needs` key, an unknown node, a `payload` that is not a join arm) keeps its text, exactly as today.
- R4 A string that holds exactly one reference and nothing else renames to the same text as before this change (the existing renaming tests keep passing unchanged).
- R5 A reference is read with the SAME character set the expression parser reads: letters, digits, `_`, `.` and `-` (`expr.rs`, `fn path`). A trailing `.` that the parser would not consume is left outside the reference.
- R6 There is ONE renaming rule: the per-reference function stays `rewrite_expr` (it is renamed only if its signature must change), and the new code finds references and calls it. No second copy of the
  nodes/input/payload arms.

- R7 The load-time checks that read a sub-graph's expressions (`graph.call.leak`, `graph.call.undeclared` in `crates/pii-graph/src/fragment/call_needs.rs`) look at EVERY reference of an expression,
  not only the first: today they split the whole string on `.`, so `$.input.k + $.nodes.host` is checked as if it read `input.k` only, and `$.input.secret + $.input.x` reads the input named `secret + $`.
  They find references with the SAME scan the renamer uses (one function that returns each reference's span; `rewrite_refs` is built on it), so the validator and the renamer cannot disagree.
  A `$.` inside a string literal is not a reference there either.
## Architecture
```
rewrite(v) for a string that starts with "$."  ->  rewrite_refs(text, refs)
rewrite_refs: scan left to right
    inside a "..." literal (with \" and \\ escapes)  -> copied verbatim
    "$." + path chars                                -> rewrite_expr(reference)            (unchanged single-reference rule)
    anything else                                    -> copied verbatim
```
Key decisions:
- Scan, do not parse. The renamer needs the text back with only the references changed; a parse and re-print would change spacing and quoting. Rejected: `Expr::parse` then re-render; the tree
  has no printer and a printer would reformat every expression in every graph.
- The scan honours string literals because the language has them and a node id is a legal word inside a literal: `"$.nodes.root"` is data.
- The same path character set as the parser, so the renamer and the validator can never disagree about where a reference ends.

## TDD Contract
New test file `crates/pii-graph/tests/call_rewrites_compound_expressions.rs`, in the style of `call_rewrites_prompt_placeholders.rs` (a sub-graph, a host that calls it, weave, read the woven node's `expr`).

| Test Name | Input | Expected Output |
|---|---|---|
| `a_concatenation_of_a_node_and_an_input_is_renamed_reference_by_reference` | sub `path`: `$.nodes.root + "/rubrics/" + $.input.rubric + ".md"`, call maps `rubric` | the woven expr names `$.nodes.docs__root` and the call's mapped input node; every literal and `+` is byte-identical |
| `a_comparison_renames_both_sides` | `$.nodes.a == $.input.k` | both sides renamed |
| `a_function_argument_is_renamed` | `lower($.nodes.a)` and `starts_with($.nodes.a, $.input.k)` | each argument renamed |
| `a_reference_inside_parentheses_is_renamed` | `($.nodes.a + "x")` | renamed |
| `CONTROL_a_reference_inside_a_string_literal_is_data` | `"$.nodes.a" + $.nodes.a` | the literal is unchanged, the reference after it is renamed |
| `CONTROL_an_escaped_quote_does_not_end_the_literal` | `"\" $.nodes.a" + $.nodes.a` | the whole first literal unchanged, the reference renamed |
| `CONTROL_a_reference_to_no_subgraph_node_keeps_its_text` | `$.nodes.zzz + $.nodes.a` | `$.nodes.zzz` unchanged, the other renamed |
| `a_host_needs_key_is_left_to_the_host_inside_a_compound` | `$.input.title + $.nodes.a` where `title` is in `needs` and not mapped | `$.input.title` unchanged, the other renamed |
| `a_single_reference_renames_exactly_as_before` | `$.nodes.a.rest` | the same text the old rule produced |
| `a_hyphenated_node_id_is_one_reference` | `$.nodes.method-judge + "x"` (node `method-judge`) | renamed as one reference |
| `the_review_graphs_path_node_evaluates_after_inlining` | `graphs/review-one.toml` called from a one-slot host, `root`=`/w`, `rubric`=`alpha` | the woven `path` node evaluates to `/w/rubrics/alpha.md` |

New test file `crates/pii-graph/tests/call_needs_reads_every_reference.rs` (R7), same weave helpers:

| Test Name | Input | Expected Output |
|---|---|---|
| `a_host_node_read_by_a_later_reference_is_a_leak` | sub node expr `$.nodes.a + $.nodes.hostnode` where `hostnode` is a host node the call does not map | refused with `graph.call.leak` naming `hostnode` |
| `an_undeclared_input_in_a_later_reference_is_refused` | `$.input.k + $.input.secret`, `secret` not declared, not mapped | refused with `graph.call.undeclared` naming input `secret` |
| `a_declared_input_in_a_later_reference_is_accepted` | the same expr with `secret` declared in `needs` | loads |
| `a_mapped_input_in_a_later_reference_is_accepted` | `$.nodes.a + $.input.k`, `k` mapped by the call | loads |
| `CONTROL_a_host_node_named_inside_a_string_literal_is_not_a_leak` | `"$.nodes.hostnode" + $.nodes.a` | loads |
| `CONTROL_a_single_reference_is_checked_as_before` | `$.nodes.hostnode` alone | refused with `graph.call.leak`, exactly as today |

Acceptance test: `a_concatenation_of_a_node_and_an_input_is_renamed_reference_by_reference`. The dangerous requirement is R2: a plausible implementation renames every `$.` it
finds with a regex, passes the positive rows, and fails only `CONTROL_a_reference_inside_a_string_literal_is_data`. A second plausible implementation scans correctly but ends a reference at
the first non-letter, so `method-judge` becomes `method` and the hyphen row catches it.

## Exit Criteria
### New in this row
Each is verified to FAIL before the change. A criterion that already passes is a Guardrail, never a criterion.
- [ ] `cargo test -q -p pii-graph --test call_rewrites_compound_expressions 2>&1 | grep -qE 'test result: ok. [1-9][0-9]* passed'`
- [ ] `cargo test -q -p pii-graph --test call_needs_reads_every_reference 2>&1 | grep -qE 'test result: ok. [1-9][0-9]* passed'`
- [ ] `grep -q 'fn reference_spans' crates/pii-graph/src/fragment/call_rewrite.rs`
## Guardrails
- Keep green (it passes now, so it is not a criterion: a criterion that passes before the work refuses the row at the RED step): `grep -q 'fn rewrite_refs' crates/pii-graph/src/fragment/call_rewrite.rs`;
  `test "$(grep -c 'fn literal_end' crates/pii-graph/src/fragment/call_rewrite.rs)" = 1` (one literal scanner, not two); `cargo test -q -p pii-graph --test call_needs`.
- Keep green (they pass today): `cargo test -q -p pii-graph --test call_inlines_a_graph`, `cargo test -q -p pii-graph --test call_rewrites_prompt_placeholders`, `cargo test -q -p pii-graph --test call_needs`,
  `cargo test -q -p pii-graph --test call_outputs_and_cycles`.
- `crates/pii-graph/src/fragment/call.rs` must not get longer: `test "$(wc -l < crates/pii-graph/src/fragment/call.rs)" -le 466`.
- `crates/pii-graph/src/fragment/call_rewrite.rs` stays under 450 lines: `test "$(wc -l < crates/pii-graph/src/fragment/call_rewrite.rs)" -lt 450`.
- Do NOT add a second renaming rule: `grep -c 'fn rewrite_expr' crates/pii-graph/src/fragment/call_rewrite.rs` stays 1.
- Do NOT touch `expr.rs`, `validate.rs` or `template.rs` (`call_needs.rs` IS in scope, for R7): the parser and validator are right; the inliner must supply names they can read.
- Do NOT edit `graphs/review.toml`, `graphs/review-one.toml` or `crates/pii-server/tests/review_graph.rs`: they belong to row 403 and are parked work, not in the tree.
- Do NOT add a node kind, an expression function or a dependency.
- Every dirty path must be listed under Files: list a file before you touch it.

## Files
- `crates/pii-graph/src/fragment/call_rewrite.rs`
- `crates/pii-graph/tests/call_rewrites_compound_expressions.rs`
- `crates/pii-graph/src/fragment/call_needs.rs`
- `crates/pii-graph/tests/call_needs_reads_every_reference.rs`
