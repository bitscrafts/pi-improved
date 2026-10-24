# Spec 255: a graph can call another graph — the `call` node, expanded inline at load

## Overview
**Project**: /projects/pi-improved
**Phase**: production — composition (operator, 2026-10-24: "graph composition is extremely needed"; "a node that calls a graph, for instance to document the task/spec")
**Advice**: an Opus design review (`/projects/_tmp/pii-graph-design/pii-graph-design-plan-20261024T155725Z.md`, Q1) chose inline expansion at load over a child run.

Today a graph cannot use another graph. `spawn` starts a child run but returns before it finishes, so its result cannot feed the next node, and the child's
caps come from its own file. `[[attach]]` (spec 212) weaves a guard at four surfaces and returns no value. The generic spec-implementation graph and its
documentation sub-graph both need "run this graph here, with this input, and continue with its result".

This spec adds `kind = "call"`: at load time the named graph's nodes are copied into the host, ids namespaced, input mapped at the call site, the sub-graph pinned
by sha256, caps checked, recursion refused. After load there is ONE graph: one run id, one journal, one cancel token, one crash-resume path.

## Verified Premises
- **CONFIRMED** `crates/pii-graph/src/fragment/weave.rs:52` `weave` parses the host TOML to a `toml::Table`, splices, then loads once through `load_toml_with_registry`; `load_host` (:88) is the entry every
  runner and checker uses (callers: `crates/pii-server/src/lib.rs:486`, `crates/pii-server/src/engine/dispatch.rs:365`, `crates/pii-cli/src/cmd_graph_installed.rs:259`). Expansion belongs in the same table-level pass, BEFORE the load.
- **CONFIRMED** `sha256_hex(&[u8]) -> String` is `pub` in `crates/pii-graph/src/fragment/check.rs:83` and re-exported from `fragment`.
- **CONFIRMED** `find_graph(dirs, name)` in `crates/pii-server/src/graph_paths.rs` resolves a graph name to `<dir>/<name>.toml`; `fragment_pin_and_nesting_refused.rs` and `fragments_attach.rs` show the test idiom for pins.
- **CONFIRMED** expressions are `$.input.<path>`, `$.payload.<path>`, `$.nodes.<id>.<path>` (`crates/pii-graph/src/expr.rs:1-15`); an unresolvable path is `Null`, not an error (:66).
- **CONFIRMED** nodes look up the blackboard by FIXED ids: `nodes.select`, `nodes.judge`, `nodes.dispatch` (`crates/pii-server/src/node_kinds/nodes/queue.rs:25,232`; `coder/helpers.rs:127`). A namespaced id breaks any such lookup inside a sub-graph; sub-graphs written for this feature must not use fixed-id lookups.
- **INFERRED, MEASURE FIRST** that a `transform` node with a `value` table evaluates nested `$.` expressions. If it does not, the `<call>__in` node must be built from one `transform` per mapped key. STOP AND ASK if neither works.
- **INFERRED** that a node id containing `__` is accepted by validation and by journal/trace. Measure with the first test; if refused, use the separator the tests prove works and say which.

## Requirements
### Functional
- **R1. Syntax.** A host node `[[node]] id = "docs" kind = "call" graph = "<name>" sha256 = "<hex>" caps = [..] [node.in] k = "<literal or $.expr>"`. `graph`, `sha256` and `caps` are required; a missing one is refused naming the key.
- **R2. Inlining.** Each node `n` of the sub-graph becomes `<call id>__<n>`. A node `<call id>__in` of kind `transform` is added; its value is the `in` table, evaluated in the HOST context. The sub-graph's entry node gets an edge from `<call id>__in`. Edges to the call node are rewritten to go to `<call id>__in`.
- **R3. Input mapping.** Inside the inlined nodes every `$.input` is rewritten to `$.nodes.<call id>__in`, and every `$.nodes.<m>` for a sub-graph node `m` to `$.nodes.<call id>__<m>`. `$.payload` is untouched. After this, the sub-graph can never read the host's run input.
- **R4. Exit.** The sub-graph declares `exit = "<node id>"` at top level (a node that exists). An edge in the host `from = "<call id>", port = P` is rewritten to `from = "<call id>__<exit>", port = P`. A sub-graph with no `exit` is refused.
- **R5. Pin.** `sha256` must equal `sha256_hex` of the sub-graph file's bytes. A mismatch is refused, printing the expected and the actual digest.
- **R6. Caps.** Every inlined node's caps must be a subset of the call node's `caps`, and the call node's `caps` a subset of the host graph's. Violation is refused naming the node and the capability. A sub-graph can never widen what the host granted.
- **R7. Recursion.** The chain of graph names is tracked. A graph that (directly or indirectly) calls itself is refused, printing the chain `a -> b -> a`. Depth is capped at 4; deeper is refused naming the cap.
- **R8. Where it runs.** Expansion happens inside `load_host`, so `pii graph check`, run start and resume all see the same expanded graph. A new `load_host_with_graphs(src, fragments_dir, graph_dirs, reg)` takes the search dirs; the old `load_host` keeps its signature and calls it with no dirs, in which case a `call` node is refused with "no graph directory". The three callers above pass the dirs `graph_search_dirs` returns.
- **R9. Not in fragments.** A `call` node inside a fragment is refused (nesting stays forbidden, as for attach).
- **R10. Unique ids.** After expansion `check_unique` still holds; a collision with a host id is refused naming both.

### Non-Functional
- No new dependency. Files stay under the 450-line gate: put the expansion in `crates/pii-graph/src/fragment/call.rs` (new), not in `weave.rs`.
- No absolute path in any new graph or fixture; temp dirs only.
- No journal or `run explain` change in this spec (recorded as a follow-up: the call pins belong in the run record).

## Architecture
```
host.toml --parse--> Table --expand_calls(graph_dirs, stack)--> Table --(existing weave of attaches)--> load_toml_with_registry --> ONE GraphSpec
                              |  for each kind="call":  read <name>.toml, check sha256, check caps, check recursion,
                              |  prefix ids  <call>__<n>,  add <call>__in (transform of the `in` table),
                              |  rewrite $.input -> $.nodes.<call>__in and $.nodes.<m> -> $.nodes.<call>__<m>,
                              |  rewrite host edges  to:<call> -> <call>__in   from:<call>,port -> <call>__<exit>,port
```

## TDD Contract
New file `crates/pii-graph/tests/call_inlines_a_graph.rs`. Names are the contract; each uses temp dirs.

| Test name | Setup | Expected |
|---|---|---|
| `a_call_node_inlines_the_subgraph` | host: `start -> docs(call) -> done`; sub: `a -> b`, `exit = "b"` | loaded spec has ids `docs__in`, `docs__a`, `docs__b`; no node `docs`; one entry |
| `the_subgraph_reads_the_mapped_input_not_the_hosts` | host input `{x: 1}`, `in.title = "$.input.x"`, sub node expr `$.input.title` | run result `1` (needs the run, not only the load) |
| `a_host_run_input_does_not_leak_into_the_subgraph` | sub node reads `$.input.x` but the call maps no `x` | result `null`, never the host's `x` (CONTROL for the test above) |
| `the_exit_port_routes_back_to_the_host` | host edge `docs --ok--> done`; sub exit emits `ok` | the host reaches `done` and the sub-graph's value is its payload |
| `a_wrong_pin_is_refused_with_both_digests` | `sha256` altered by one hex digit | error contains the expected AND the actual digest |
| `a_correct_pin_loads` | right digest | loads (CONTROL for the refusal) |
| `a_subgraph_cannot_widen_caps` | sub node needs `spawn`, the call node grants only `proc` | error names the node and `spawn` |
| `a_subgraph_within_the_grant_loads` | same sub, call node grants `spawn` and host grants `spawn` | loads (CONTROL) |
| `a_graph_that_calls_itself_is_refused` | `a` calls `a` | error contains `a -> a` |
| `an_indirect_cycle_is_refused_with_the_chain` | `a` calls `b`, `b` calls `a` | error contains `a -> b -> a` |
| `depth_five_is_refused_and_depth_four_loads` | chain of 5 graphs, then 4 | the 5-chain is refused naming the cap, the 4-chain loads |
| `a_subgraph_without_an_exit_is_refused` | no `exit` key | error names `exit` |
| `a_call_without_graph_sha256_or_caps_is_refused` | each key removed in turn (3 cases) | error names the missing key each time |
| `a_call_in_a_fragment_is_refused` | fragment containing a `call` node | refused |
| `the_old_load_host_refuses_a_call_with_no_directory` | `load_host` (no dirs) | error contains `no graph directory` |
| `an_id_collision_is_refused` | host already has a node `docs__a` | error names both |
| `a_graph_with_no_call_loads_byte_identically` | an existing fixture graph | `load_host_with_graphs` and `load_host` give equal specs (CONTROL: composition changes nothing for graphs that do not use it) |

**The requirement a plausible implementation satisfies only in appearance:** rewriting the node ids but not the `$.nodes.<m>` references inside the inlined nodes, or not `$.input`. The graph loads and `check` passes, and the sub-graph silently reads the host's input or a null. `the_subgraph_reads_the_mapped_input_not_the_hosts` and `a_host_run_input_does_not_leak_into_the_subgraph` exist for exactly this; both must RUN the graph, not only load it.

## Exit Criteria
- [ ] `cargo test -q -p pii-graph --test call_inlines_a_graph 2>&1 | grep -qE "^test result: ok\. (1[7-9]|[2-9][0-9]) passed"`
- [ ] `cargo test -q -p pii-graph --test fragments_attach 2>&1 | grep -qE "^test result: ok"`
- [ ] `cargo test -q -p pii-graph --test fragment_pin_and_nesting_refused 2>&1 | grep -qE "^test result: ok"`
- [ ] `cargo test -q -p pii-graph > /dev/null 2>&1`
- [ ] `cargo test -q -p pii-server --lib 2>&1 | grep -qE "^test result: ok"`
- [ ] `cargo clippy -q -p pii-graph -p pii-server -p pii-cli --all-targets -- -D warnings`
- [ ] `test -f crates/pii-graph/src/fragment/call.rs`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

## Guardrails
- Do NOT add a child-run mechanism, a new node-kind in `pii-server`, or change `spawn`.
- Do NOT change how `[[attach]]` weaves, or the signature of `load_host`.
- Do NOT edit any existing test; a failing existing test is a finding to report.
- Do NOT put a fixed-id blackboard lookup in any new graph or fixture.
- Do NOT commit.

## Follow-ups (not this spec)
The call pins in the run record and `run explain`; a `call` whose graph name comes from the run input (that needs an awaited child run with a caps intersection, the case where the advice would choose differently); retiring the fixed-id lookups in `queue.rs`.
