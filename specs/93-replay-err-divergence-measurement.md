# Spec: 93 — measure whether replay's discarded `apply_completion` error changes state (spec 91 finding 6)

**Project**: /projects/pi-improved
**Phase**: 045 follow-through, finding 6 — the one remaining item that could be a REAL defect rather than
a test gap. **This spec measures first; it does not patch.**

## Context
Spec 91's allow-review of `crates/pii-graph/src/run_helpers.rs:125`
(`let _ = apply_completion(...)` in `replay`) raised a specific doubt. On `Ok` the discard is fine (state
mutation only). On `Err` — the loop-budget-exceeded / re-entry paths — `apply_completion` returns BEFORE
firing the node's outbound edges (`state.rs` ~213-231, the `it == b` / `it > b` returns), so a replayed
graph can differ from the live run in which edges were activated. The comment says the activation outcome
is irrelevant, which holds only if the failure is recovered elsewhere; replay recovers a failure only
from `NodeFailed` events for nodes not in `finished`, and a node with a `NodeFinished` is excluded from
that — so it is plausible that a genuine divergence exists. Nobody has measured it.

## Requirements
R1. Build a MINIMAL reproduction in `crates/pii-graph/tests/` (or a `#[cfg(test)]` module) with a graph
    that (a) finishes a node, (b) then drives `apply_completion` into its `Err` path on a path that
    matters (loop budget exceeded is the named candidate — pick whichever Err path is reachable and say
    which one and why), and (c) is then replayed from the journal.
R2. MEASURE, do not assert a belief: report one of exactly two outcomes with evidence:
    - **DIVERGENCE**: the replayed run's activated-edge set / final state differs from the live run's.
      Give both values verbatim (the live and the replayed sets) and the minimal step where they part.
    - **NO DIVERGENCE**: the states are equal, and say WHY the Err path cannot matter here (what
      recovers it).
    3. Either outcome must come from running the code, not from reading it: paste the test name, the
    run command, and its output.
R3. If DIVERGENCE is found, do NOT fix it. Write the smallest honest reproduction as a perm...[truncated]