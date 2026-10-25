# Spec 267: a declared `needs` key that resolves to nothing refuses the run, naming the key and the call

## Overview
**Project**: /projects/pi-improved
**Phase**: production — composition (spec 261 follow-up, Opus context review 2026-10-24, risk 2: "silent Null for a missing key")

Spec 261 checks `needs` at LOAD: an undeclared read is refused. It does nothing at RUN time. A sub-graph that declares `needs = ["spec"]` and is called from a run whose input has no `spec` still starts, reads `$.input.spec`, gets `Null` (`crates/pii-graph/src/expr.rs`: an unresolvable path is `Null`, not an error) and carries on: a docs coder is briefed about "null". The contract is only worth having if a declared key that is not there stops the run before any sub-graph node runs.

A `route` node cannot do it: its rules are written in TOML, TOML has no null, so `{ eq = null }` cannot be written. It needs a small node kind.

## Verified Premises
- **CONFIRMED** (read, 2026-10-24) `grep -rn "graph.call.need" crates` finds NO run-time refusal; only `graph.call.needs` (load-time, malformed declaration) in `call_needs.rs:30` and `tests/call_needs.rs:235`.
- **CONFIRMED** (read) `crates/pii-graph/src/node.rs:77-87` `NodeRegistry::builtin()` registers six kinds (`transform route emit wait timer refuse`); a new built-in kind is one `map.insert` plus a factory.
- **CONFIRMED** (read) `crates/pii-graph/src/nodes.rs:264-270`: `RefuseNode::run` returns `Outcome::Fail { error }`; that is how a node ends a run, and the existing refusals journal their text.
- **CONFIRMED** (read) `crates/pii-graph/src/fragment/call.rs:251-312` `splice` builds the chain `<call>__in` (head) → one `<call>__in__<key>` transform per mapped key → `<call>__<entry>`. It is 446 lines; the gate allows 450. Chain construction for the new node therefore goes in `call_needs.rs` (140 lines), and `call.rs` may grow by at most 3 lines.
- **CONFIRMED** (read) `eval` of an absent path gives `Null`; a mapped key whose value expression is absent also gives `Null` in its `<call>__in__<key>` transform output.
- **NOT CLAIMED** that an empty string or `false` is "missing": only `Null` and an absent path are.

## Requirements
### Functional
- **R1. A `require` node kind.** New built-in kind `require` in a NEW file `crates/pii-graph/src/nodes_require.rs`, registered in `NodeRegistry::builtin()`. Config: `keys` = a table of `<name> = "<expression>"`, plus `call` and `graph` (strings, used only in the message). Run: evaluate every expression like `transform` does (same evaluator), in the table's key order; the FIRST that is `Null` (or fails to resolve) returns `Outcome::Fail` with the text `graph.call.need: call '<call>' graph '<graph>' needs '<name>'`. Otherwise `Outcome::Next` on port `ok` with the input value unchanged. A config without `keys`, or `keys` not a table, is refused at build with `PiiError::Invalid`.
- **R2. The chain gets it, only when `needs` is declared.** In a sub-graph that declares `needs`, `call_needs.rs` provides `require_node(id, graph, needs, mapped_keys) -> Table` and the call's chain gains one `require` node `<call>__need` placed AFTER the last `<call>__in__<key>` transform and BEFORE the sub-graph entry. For each declared key: if the call's `in` maps it, the expression is `$.nodes.<call>__in__<key>`; otherwise it is `$.input.<key>`. A sub-graph without `needs` gets NO extra node: its expanded graph is byte-for-byte what spec 261 produces today (compare the expanded node ids).
- **R3. Order and reason.** The refusal happens before any sub-graph node runs (the `require` node sits ahead of the entry), and its text names the key and the call, so a person reading `run explain` sees which key of which call was missing.
- **R4. Tests, with controls.** New `crates/pii-graph/tests/call_requires_needs.rs` (a new file; do NOT edit `call_needs.rs` or `call_inlines_a_graph.rs`). Each test loads a host with a real temp graph dir and RUNS the expanded graph with the in-process scheduler (copy the helper style of `call_needs_holder.rs`): (a) declared key present in the host input → the run succeeds and the sub-graph node saw the value; (b) declared key absent from the host input → the run fails with the exact `graph.call.need` text naming key and call, and the sub-graph's node did NOT run (assert its output is absent from the blackboard); (c) declared key mapped by `in` to an expression that resolves → succeeds; (d) declared key mapped to an expression that resolves to Null → refused naming that key; (e) two declared keys, the first present and the second absent → refused naming the second; (f) CONTROL: a sub-graph with NO `needs` and a read of an absent `$.input.x` still runs as before (no refusal, no `__need` node in the expanded graph); (g) CONTROL: an empty string for a declared key is accepted (only Null refuses); (h) `require` with a missing `keys` is refused at build; (i) the expanded graph's node ids for a `needs` sub-graph include `<call>__need` and for a plain one do not.

### Non-Functional
- Keep every file under the 450-line gate; `call.rs` stays at or below 450. No new dependency. Do not change `call_needs.rs`'s existing load-time checks, only add `require_node`.
- Do not edit `crates/pii-graph/tests/call_needs.rs`, `call_inlines_a_graph.rs`, `call_needs_holder.rs`, `spec_graph_is_tracked_and_valid.rs`.

## TDD Contract
| Test (`call_requires_needs.rs`) | Expected |
|---|---|
| `a_present_key_runs_the_subgraph` | run succeeds; the node saw the host value |
| `an_absent_key_refuses_before_any_subgraph_node` | `Fail` text `graph.call.need: call 'docs' graph 'note' needs 'spec'`; sub-graph output absent |
| `a_mapped_key_that_resolves_runs` | succeeds |
| `a_mapped_key_that_is_null_refuses` | refused naming the key |
| `the_second_missing_key_is_the_one_named` | names the second key |
| `CONTROL_without_needs_nothing_changes` | runs; no `docs__need` node |
| `CONTROL_an_empty_string_is_accepted` | succeeds |
| `a_require_without_keys_is_refused_at_build` | `PiiError::Invalid` |
| `the_need_node_exists_only_with_needs` | ids as stated |

**The requirement a plausible implementation satisfies only in appearance:** putting the check in the head `<call>__in` or checking only the unmapped keys. A mapped key whose expression resolves to Null passes a check that reads only host input, and a check ahead of the `in__<key>` transforms cannot see mapped values at all. Test (d) fails for both.

## Exit Criteria
- [ ] `cargo test -q -p pii-graph --test call_requires_needs 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `test -f crates/pii-graph/src/nodes_require.rs`
- [ ] `grep -q "graph.call.need:" crates/pii-graph/src/nodes_require.rs`
- [ ] `grep -q "fn require_node" crates/pii-graph/src/fragment/call_needs.rs`
- [ ] `cargo test -q -p pii-graph --test call_needs 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `cargo test -q -p pii-graph --test call_inlines_a_graph 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `cargo test -q -p pii-graph --test call_needs_holder 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `git diff --quiet -- crates/pii-graph/tests/call_needs.rs crates/pii-graph/tests/call_inlines_a_graph.rs crates/pii-graph/tests/call_needs_holder.rs crates/pii-graph/tests/spec_graph_is_tracked_and_valid.rs`
- [ ] `cargo clippy -q -p pii-graph --all-targets -- -D warnings`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

## Guardrails
- Do NOT edit the four test files named above. Do NOT make a sub-graph without `needs` get a `require` node. Do NOT treat an empty string, `false` or `0` as missing. Do NOT commit.

## Follow-ups (not this spec)
Journal the call boundary (`graph.call` event with pins, keys, each key's source); per-node pi sessions; `graphs/docs.toml`; the coder charging the run budget.
