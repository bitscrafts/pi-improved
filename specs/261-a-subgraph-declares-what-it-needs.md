# Spec 261: a called graph declares what it needs from the host — `needs`, checked at load

## Overview
**Project**: /projects/pi-improved
**Phase**: production — composition, PRIORITY (operator, 2026-10-24: "our graphs must run subgraphs sharing context! priority on that!"; then "auto accept suggestions")
**Advice**: Opus design review `/projects/_tmp/pii-graph-context/pii-graph-context-plan-20261024T193127Z.md` (session `88f556e2-5d1b-49e6-b56e-e49b6cb84667`) REPLACED the first draft of this spec (a `context = "shared"` key on the call).

Spec 255 expands a `call` inline at load: ONE graph, one run id, one journal, one cancel token. Its isolation is lexical only: it rewrites `$.` strings, so a sub-graph sees only what the call maps in (`in`).
The operator needs the opposite for the real use cases (a documentation sub-graph, a judge, a fix step): the sub-graph must work on the run's own context (`spec`, `base`, `workdir`, `agent`…) without the host re-listing every key.

The contract belongs on the SUB-GRAPH: it declares the run-input keys it needs; the loader checks every `$.input.<k>` it reads against that list; a key the call does not map falls through to the host's run input.
A call-side switch (the first draft) lets a host hand over more than the sub-graph asked for, and cannot be enforced (see Verified Premises). A sub-graph WITHOUT the declaration keeps spec 255's behaviour byte for byte.

## Verified Premises
- **CONFIRMED** (read, `crates/pii-graph/src/fragment/call_rewrite.rs`) `rewrite_expr`: `$.input.<k>` for a key the call maps goes to `$.nodes.<call>__in__<k>`; any other `$.input…` goes to `$.nodes.<call>__in…` (so it reads Null); `$.nodes.<m>` for a SUB-GRAPH node `m` goes to `<call>__<m>`; every other expression falls through unchanged (`_ => expr.to_string()`), so a sub-graph CAN read a host node by id today.
- **CONFIRMED** (read, `crates/pii-graph/src/sched/dispatch.rs`, `build_node_input`) every node, inlined or not, receives `input: bb.input.clone()` (the host's run input) and the whole blackboard; a node with no inbound Data edge receives `bb.input` as `payload`; a node with ONE inbound Data edge receives that edge's value directly as `payload`; a node with SEVERAL inbound Data edges (a join) receives `payload` as an object keyed by the SOURCE node id. So the rewrite governs `$.` strings only; Rust-side reads of `NodeInput.input` (the coder's `scope_root`, `heal_record`'s `signature`/`gate`) see the host input regardless. This spec does not change that and says so in the module comment (R9).
- **CONFIRMED** (read, `call.rs`) the call table's keys read are `id`, `graph`, `sha256`, `caps`, `in` (`expand_one`); any other key on a call is silently ignored (a `loop_budget` on a call vanishes). The head is a `transform` node `<call>__in` followed by one `transform` per `in` key (`<call>__in__<k>`), chained by edges.
- **CONFIRMED** (read, `crates/pii-graph/tests/call_inlines_a_graph.rs`) the 19 existing fixtures read only sub-graph nodes (`$.nodes.a` where `a` is the sub-graph's own node), give calls only the keys `id kind graph sha256 caps in`, and read `$.payload` only in the HOST (`done`). So R6 and R8 below cannot break them. If one breaks anyway: that is a finding, report it in the final message and leave the test untouched.
- **CONFIRMED** transform nodes in the fixtures use `expr = "..."` (and `value = "..."` for a literal).
- **CONFIRMED** (run) 19 tests in `call_inlines_a_graph.rs` pass today, including `a_host_run_input_does_not_leak_into_the_subgraph`.
- **NOT IN THIS SPEC** (Opus, recorded as follow-ups): the run-time check that a declared key resolved (spec 262), the journalled `graph.call` event (263), per-node agent sessions (264), `graphs/docs.toml` (265), the coder charging the run budget (266).

## Requirements
### Functional
- **R1. The declaration.** A sub-graph may carry a top-level `needs = ["<key>", ...]`. It must be an array of non-empty strings with no duplicates; anything else is refused at load with `graph.call.needs: graph '<g>' has a malformed 'needs'`. Absent: spec 255's behaviour, byte for byte, for every call to that graph.
- **R2. A declared key falls through.** In a sub-graph that declares `needs`, `$.input.<k>.<rest…>` for a declared key `<k>` that the call's `in` does NOT map is left unrewritten: it reads the host run's input.
- **R3. A mapped key still overrides.** A key that appears in the call's `in` is read from the mapped value (`$.nodes.<call>__in__<k>`), declared or not.
- **R4. An undeclared key is refused.** In a sub-graph that declares `needs`, a `$.input.<k>` read where `<k>` is neither declared nor mapped is refused at load: `graph.call.undeclared: graph '<g>' node '<n>' reads input '<k>' but declares only [<needs>] and call '<id>' maps [<keys>]`. Bare `$.input` (no key) in such a sub-graph is refused the same way (`reads the whole input`), because it would bypass the declaration.
- **R5. A host node is never readable by id.** In EVERY sub-graph (with or without `needs`), a `$.nodes.<x>` read where `<x>` is a node of the HOST graph and not a node of the sub-graph is refused: `graph.call.leak: graph '<g>' node '<n>' reads host node '<x>' through call '<id>'; map it with 'in'`. A host node's output reaches a sub-graph only through `in` (`in.summary = "$.nodes.code.summary"`, evaluated in the host's context as today). An id that is neither a host node nor a sub-graph node is left as today (it resolves to Null).
- **R6. A join keeps its arms.** Inside a sub-graph, a node with MORE THAN ONE inbound Data edge reads `$.payload.<m>` where `<m>` is the source of one of those edges: after inlining the source id is `<call>__<m>`, so the read is rewritten to `$.payload.<call>__<m>`. A node with one or no inbound Data edge is NOT rewritten (its payload is a direct value, and a field that happens to share a node's name must stay untouched). This fixes a confirmed spec 255 bug in both modes.
- **R7. Unknown keys on a call are refused.** A call node with a key other than `id kind graph sha256 caps in` is refused: `graph.call.key: call '<id>' has unknown key '<k>'`. (A silently dropped `loop_budget` is worse than a refusal.)
- **R8. Nesting is transitive.** The check runs for each call as the loader expands it, innermost first. An inner sub-graph's declared-but-unmapped key becomes a `$.input.<k>` read of the sub-graph that holds the inner call, and R4 then requires the outer sub-graph to declare it (or map it): a chain reads the outermost host's input only through declarations at every level.
- **R9. Nothing else changes.** The sha256 pin, the caps ceiling (sub ⊆ call ⊆ host), recursion and the depth cap of 4 behave exactly as in spec 255, with or without `needs`. The module comments of `call.rs` and `call_rewrite.rs` are corrected: they say the rewrite governs `$.` expressions only, and name the Rust-side reads that see the host input (`coder` `scope_root`, `heal_record`) and the no-inbound-edge payload. `specs/255-*.md` is not edited.
- **R10. One run.** A composed graph with `needs` loads to ONE `GraphSpec` with one entry and no `call` node.

### Non-Functional
- No new dependency; files stay under the 450 effective-line gate. `call.rs` is 415 lines: put the new logic in `call_rewrite.rs` (48 lines) and a new `crates/pii-graph/src/fragment/call_needs.rs`, and let `call.rs` grow by at most 15 lines.
- No change to `weave.rs`, `[[attach]]`, the daemon, the CLI, or any existing test. No absolute path in a fixture; temp dirs only.

## Architecture
```
sub-graph file:  name = "docs"   exit = "fmt"   needs = ["spec", "workdir"]      # NEW, optional
host:            [[node]] id = "docs" kind = "call" graph = "docs" sha256 = ".." caps = [..]  [node.in] summary = "$.nodes.code.summary"

inside the sub-graph, per $. read (call "docs"):
  $.input.spec          declared, not mapped  -> UNCHANGED  (the host run's input)                         R2
  $.input.summary       mapped by `in`        -> $.nodes.docs__in__summary                                  R3
  $.input.other         neither               -> REFUSED graph.call.undeclared                              R4
  $.input               bare                  -> REFUSED graph.call.undeclared                              R4
  $.nodes.fmt           a sub-graph node      -> $.nodes.docs__fmt                                          (as today)
  $.nodes.code          a HOST node           -> REFUSED graph.call.leak                                    R5
  $.payload.<arm>       in a JOIN             -> $.payload.docs__<arm>                                      R6
```

## TDD Contract
New file `crates/pii-graph/tests/call_needs.rs`. It copies the few helpers it needs from `call_inlines_a_graph.rs` (temp dirs, `load`, `run_spec`, `output_of`); it does NOT edit that file. Tests that read a value RUN the graph.

| Test name | Setup | Expected |
|---|---|---|
| `a_declared_key_reads_the_host_input` | sub `needs=["x"]`, node reads `$.input.x`; call has NO `in`; host input `{x: 7}` | run output `7` |
| `a_graph_without_needs_stays_isolated` | same sub without `needs` | output `null` (CONTROL for the test above) |
| `a_mapped_key_overrides_a_declared_one` | `needs=["x","y"]`, `in.x = "mapped"`, input `{x: 7, y: 8}`; node A reads `$.input.x`, node B reads `$.input.y` | A is `"mapped"`, B is `8` |
| `an_undeclared_key_is_refused_naming_graph_node_and_key` | `needs=["x"]`, a node reads `$.input.y`, no `in` | load error contains `graph.call.undeclared`, the graph name, the node id, `y` |
| `bare_input_is_refused_when_needs_is_declared` | `needs=["x"]`, a node reads `$.input` | load error contains `graph.call.undeclared` and `whole input` |
| `a_malformed_needs_is_refused` | `needs = "x"`; then `needs = [1]`; then `needs = ["a","a"]`; then `needs = [""]` | each: `graph.call.needs` |
| `a_host_node_read_is_refused_with_needs` | `needs=["x"]`, sub node reads `$.nodes.first` where `first` is a host node | `graph.call.leak`, `first`, the call id |
| `a_host_node_read_is_refused_without_needs` | same, no `needs` | `graph.call.leak` (the confirmed hole is closed in both modes) |
| `a_sub_graph_node_read_is_still_namespaced` | sub node reads its own `$.nodes.a` | loads and runs (CONTROL: R5 does not refuse the sub-graph's own nodes) |
| `a_host_node_reaches_the_subgraph_through_in` | `in.v = "$.nodes.first"`; sub reads `$.input.v` | output is `first`'s value |
| `a_join_reads_both_arms` | sub: two parallel nodes `l` and `r` feeding a join `j` whose expr builds from `$.payload.l` and `$.payload.r` | both values present in the output |
| `a_single_inbound_node_is_not_rewritten` | a node with ONE inbound edge reads `$.payload.l` where `l` is also a sub-graph node id and the upstream value has a field `l` | it reads the FIELD (CONTROL for the join rewrite) |
| `an_unknown_call_key_is_refused` | `loop_budget = 3` on the call | `graph.call.key` and `loop_budget` |
| `the_known_call_keys_still_load` | the call with exactly `id kind graph sha256 caps in` | loads (CONTROL) |
| `a_nested_need_must_be_declared_by_the_outer_graph` | host calls `mid` (`needs=[]`) which calls `leaf` (`needs=["x"]`, no `in`) | `graph.call.undeclared` naming `mid` |
| `a_nested_chain_reads_the_outermost_input` | `mid` `needs=["x"]`, `leaf` `needs=["x"]`, host input `{x: 7}` | output `7` |
| `a_wrong_pin_is_still_refused_with_needs` | pin of 64 zeros | `fragment.pin.mismatch` (CONTROL: `needs` is not a bypass) |
| `needs_does_not_widen_caps` | sub node needs `proc`, call grants `[]`, sub has `needs` | refused as in spec 255 (CONTROL) |
| `a_graph_without_needs_loads_identically` | the HOST/SUB fixtures of spec 255 | the loaded `GraphSpec` equals what the spec 255 loader gave (CONTROL: byte-identical for graphs that never use `needs`) |
| `the_loaded_graph_has_no_call_node_and_one_entry` | a composed graph with `needs` | no node of kind `call`, exactly one entry |

**The requirement a plausible implementation satisfies only in appearance:** leaving `$.input.<k>` unrewritten for ALL keys once `needs` is present (the easy way to make the happy path pass). `a_declared_key_reads_the_host_input` passes either way; `an_undeclared_key_is_refused_naming_graph_node_and_key`, `bare_input_is_refused_when_needs_is_declared` and `a_mapped_key_overrides_a_declared_one` are the three that catch it, and each must run or load the real fixture.

## Exit Criteria
- [ ] `cargo test -q -p pii-graph --test call_needs 2>&1 | grep -qE "^test result: ok\. (19|[2-9][0-9]) passed"`
- [ ] `cargo test -q -p pii-graph --test call_inlines_a_graph 2>&1 | grep -qE "^test result: ok\. 19 passed"`
- [ ] `git diff --quiet -- crates/pii-graph/tests/call_inlines_a_graph.rs crates/pii-graph/src/fragment/weave.rs specs/255-a-graph-can-call-another-graph.md`
- [ ] `test -f crates/pii-graph/src/fragment/call_needs.rs`
- [ ] `grep -q "graph.call.undeclared" crates/pii-graph/src/fragment/call_needs.rs`
- [ ] `grep -q "graph.call.leak" crates/pii-graph/src/fragment/call_needs.rs`
- [ ] `grep -q "graph.call.key" crates/pii-graph/src/fragment/call.rs`
- [ ] `cargo test -q -p pii-graph 2>&1 | grep -c "test result: FAILED" | grep -qx 0`
- [ ] `cargo clippy -q -p pii-graph --all-targets -- -D warnings`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

## Guardrails
- Do NOT edit `crates/pii-graph/tests/call_inlines_a_graph.rs`, `weave.rs`, `specs/255-*.md`, or any other existing test; a failing existing test is a finding to report, not a test to change.
- Do NOT add a `context` key, a default mode, or any way for a sub-graph to write into a HOST node's id.
- Do NOT add a dependency, a raw print macro, or an absolute path.
- Do NOT commit.

## Follow-ups (not this spec)
262 the run-time check that a declared key resolved (`graph.call.need`); 263 the journalled `graph.call` event and `run explain`; 264 per-node agent sessions with explicit sharing; 265 `graphs/docs.toml` called from `spec.toml`; 266 the coder charges the run budget (gates any fix loop); the fixed-id lookups in `heal_record`/`queue` written as `$.nodes.<id>` expressions.
