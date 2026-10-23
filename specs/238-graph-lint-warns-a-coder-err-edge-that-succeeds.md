# Spec: 238 — graph lint warns when a coder's err edge ends in a node that succeeds

**Project**: /projects/pi-improved
**Phase**: filed 2026-10-23T03:00Z by the orchestrator. Premises read: `crates/pii-graph/src/validate.rs` holds the
graph lint and builds `warnings` (V5 at lines ~162-210 is the closest model: a node with a declared port and no outgoing
edge). Measured on 2026-10-23 with `graphs/claude-probe.toml` and `graphs/coder-selffix.toml`: the `coder` node routes a
non-zero exit to its `err` port, the `err` edge goes to a `transform` node with no outgoing edge, and the run finished
`succeeded` while the agent had exited 1 (runs r000000000641 and r000000000706).

## Overview

A coder agent that fails is routed correctly to `err`. If the graph ends that edge in a `transform` terminal, the run
completes successfully, so a failed agent reads as a green run. The graph author sees no warning. This spec adds a lint
WARNING only; it changes no engine semantics and fails no graph.

## Requirements

- **R1** The lint emits one warning per offending coder node, text beginning with the next free `V` number (read the
  existing ones in `validate.rs` and use the next; do not reuse one), naming the node id and the target node id, saying the
  `err` edge ends in a node of kind `transform` with no outgoing edge, so a failed agent finishes the run as succeeded.
- **R2** It fires only for a node of kind `coder` whose `err` port has an edge to a `transform` node that itself has no
  outgoing edge.
- **R3** It does NOT fire when: the `err` edge goes to any other node kind; the target `transform` has an outgoing edge; the
  coder has no `err` edge; or the node is not a coder.
- **R4** Existing warnings and the existing lint tests are unchanged.

## TDD Contract

New test file `crates/pii-graph/tests/coder_err_edge_lint.rs`, building small graphs the way the neighbouring lint tests do
(read one first and follow its construction).

| test | given | expects |
|---|---|---|
| `a_coder_err_edge_to_a_terminal_transform_warns` | coder -err-> transform, transform has no outgoing edge | one warning naming both ids |
| `an_err_edge_to_a_node_that_continues_does_not_warn` | the transform has an outgoing edge | none (CONTROL) |
| `an_err_edge_to_another_kind_does_not_warn` | err edge to a non-transform node | none (CONTROL) |
| `a_coder_with_no_err_edge_does_not_warn` | only an `ok` edge | none (CONTROL) |
| `a_non_coder_node_does_not_warn` | a transform with an `err` port edge to a terminal transform | none (CONTROL) |
| `the_shipped_probe_graph_warns` | parse `graphs/claude-probe.toml` | exactly one such warning |

The requirement a plausible implementation gets wrong is R3: matching "any node whose err edge ends in a terminal" would warn
on non-coder graphs (the `queue` and digest graphs) and drown the signal. The CONTROL rows catch it.

## Exit Criteria

- [ ] `cargo test -q -p pii-graph --test coder_err_edge_lint 2>&1 | grep -qE "^test result: ok\. [1-9]"`
- [ ] `cargo run -q -p pii-cli --bin pii -- graph lint graphs/claude-probe.toml 2>&1 | grep -q "finishes the run as succeeded"`
- [ ] `! cargo run -q -p pii-cli --bin pii -- graph lint graphs/queue.toml 2>&1 | grep -q "finishes the run as succeeded"`
- [ ] `cargo test -q -p pii-graph --lib 2>&1 | grep -qE "^test result: ok\. [1-9]"`
- [ ] `cargo clippy -q -p pii-graph --all-targets -- -D warnings`

## Guardrails

- `specs/` are READ-ONLY for the implementer. If `validate.rs` does not have the shape described (no `warnings` vector, no
  edge/port model), STOP and report the mismatch instead of forcing the change.
- This is a WARNING. Never make the lint fail a graph, never change a run's status, never edit a shipped graph.
- Never weaken a test. The wording `finishes the run as succeeded` is part of the contract (criteria 2 and 3 grep it).
- Tests write under `_tmp/`. 450-effective-line file gate and `#![forbid(unsafe_code)]` apply.
- Do NOT commit; the orchestrator commits.

## Closure

_(empty — the implementer's readings go here)_
