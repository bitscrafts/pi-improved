# Spec: A graph includes a graph — composition by name and pin

## Overview

Composability is pii's most important feature, and today it stops one level short. A graph is a set of nodes; a
capability that is missing is expressed as another **graph**, and there is no way for one graph to name another. So
every assembly (the night, the SDD loop, the TDD loop, the digest) re-declares the same nodes instead of composing the
graphs that already exist, and each copy drifts.

This spec makes graph-includes-graph first class: a node that says *"run this graph, by name, at this pin, with these
inputs"* — resolved through the same registry and the same capability ceiling, so composition is not an escape hatch.

The operator's picture, in their words: *"one graph that does tdd, one graph that does sdd, and other graphs compose
them!"* — and *"like a puzzle!"*.

**Project**: /projects/pi-improved
**Phase**: composability, after spec 212 (fragments) and spec 215 (capability ceiling)

## Requirements

### Functional

- **A new node kind, `graph`** — the one kind this feature is allowed to add, because it *is* the feature:
  ```toml
  [[node]]
  id = "sdd"
  kind = "graph"
  graph = "queue"                 # resolved from the graph search path (graphs/, then the daemon's graph dir)
  pin = "sha256:5f0c…"            # the graph file's hash; a mismatch refuses the run at LOAD, not at run time
  input = { row = "$.pick.row" }  # maps parent values into the child's input
  output = ["verdict"]            # which of the child's terminal outputs appear on the blackboard under `sdd`
  ```
- **Resolution**: `graph` + `pin` are both mandatory. A name without a pin is refused ("an unpinned include is a
  dependency you did not record"); a pin that does not match the file's bytes is refused at load with both hashes
  named. This is the same discipline fragments already follow (spec 212: attach by name with a sha256).
- **Inputs and outputs** are explicit: the child sees only what the parent maps in; the parent sees only the outputs it
  names. No shared mutable state between a parent and a child — the blackboard is per run and the mapping is the
  contract.
- **Capabilities**: the child's `[caps]` are intersected with the parent's ceiling (spec 215). A child asking beyond it
  is refused at load, with the offending capability named — composition may not escalate.
- **Cycles**: a graph that includes itself, directly or transitively, is refused at load with the cycle's path printed
  (`a → b → a`). A repeated include of the same graph at the same pin in a *non-cyclic* position is allowed and is one
  node each.
- **A child failure fails the parent's node**, carrying the child's run id and the failing node id, so evidence
  survives one level up.
- The child runs as its own **run** — same journal, same resume semantics: a parent waiting on a child that suspended
  on a timer suspends too, and resuming the parent resumes the child at the same node.

### Non-Functional

- No change to the meaning of any existing kind: `graph` composes, it does not re-implement.
- `pii graph lint` gains three refusals: an unpinned include; a pin mismatch; a capability escalation. Each names the
  file, the line, and the two values that disagree.
- Depth and breadth are bounded by the run's existing `max_wall_ms` and node budget; an include does not reset them.
- The composition is visible in `pii run get <id>`: the child run ids appear under the including node.

## Architecture

```
graphs/night.toml
  clock(timer) -> pick(select) -> sdd(  kind="graph", graph="queue", pin=sha256:…  )   <- composition
                                     └─ queue.toml: select -> coder -> judge -> record
  sdd -> review( kind="graph", graph="spec", pin=sha256:… )  -> post

load:   resolve name -> read bytes -> compare pin -> parse -> intersect caps -> build child registry view
run:    parent node = child run; journal both; a suspend propagates; a failure carries child run id + node id
```

## Files

- `specs/343-a-graph-includes-a-graph.md`
- `crates/pii-graph/src/include.rs`
- `crates/pii-server/src/node_kinds/nodes/graph.rs`
- `crates/pii-server/tests/graph_include.rs`

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| an_unpinned_include_is_refused | `kind="graph"` with no `pin` | load error naming the file and line |
| a_pin_mismatch_is_refused_with_both_hashes | pin of a different file | error printing expected and actual |
| a_child_cannot_exceed_the_parents_ceiling | child asking `proc` under a parent without it | error naming the capability |
| a_cycle_is_refused_with_its_path | a → b → a | error printing `a → b → a` |
| child_outputs_appear_under_the_node_id | child emitting `verdict` | `$.nodes.sdd.verdict` in the parent |
| a_child_failure_names_both_ids | child failing at node `judge` | parent node fails, error carries child run id and `judge` |
| a_suspended_child_suspends_the_parent | child suspended on a timer | parent suspended, same run tree |
| resuming_the_parent_resumes_the_child | resume after the timer | child continues at the node it stopped on |

## Exit Criteria

- [ ] `cargo test -q -p pii-server --test graph_include`
- [ ] `pii graph lint graphs/night.toml`
- [ ] `cargo test -q -p pii-graph --test include_resolution`

## Guardrails

- Do NOT let an include be unpinned or resolved by a bare name: a composition you cannot name by hash is a copy with
  extra steps.
- Do NOT share mutable state between parent and child: the input/output mapping is the whole contract.
- Do NOT let a child reset the run's budget, depth or capability ceiling.
- Do NOT implement this by shelling out to `pii run start`: that would make composition a subprocess, not a graph.
- Do NOT delete `graphs/queue.toml` or `graphs/spec.toml` to "clean up": they are the pieces this feature exists to
  compose.
