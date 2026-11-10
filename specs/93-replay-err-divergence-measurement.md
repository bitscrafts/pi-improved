# Spec: 93 — measure whether replay's discarded `apply_completion` error changes state (spec 91 finding 6)

**Project**: <repo>
**Phase**: 045 follow-through, finding 6 — the one remaining item that could be a REAL defect rather than
a test gap. **This spec measures first; it does not patch.**

## Context
Spec 91's allow-review of `crates/pii-graph/src/run_helpers.rs:125`
(`let _ = apply_completion(...)` in `replay`) raised a specific doubt. On `Ok` the discard is fine (state
mutation only). On `Err` — the loop-budget-exceeded / re-entry paths — `apply_completion` returns BEFORE
firing the node's outbound edges (`sched/state.rs` ~213-231, the `it == b` / `it > b` returns), so a
replayed graph can differ from the live run in which edges were activated. The comment says the
activation outcome is irrelevant, which holds only if the failure is recovered elsewhere; replay recovers
a failure only from `NodeFailed` events for nodes not in `finished`, and a node with a `NodeFinished` is
excluded from that — so a genuine divergence is plausible. Nobody has measured it.

## Requirements
R1. Build a MINIMAL reproduction in `crates/pii-graph/tests/` with a graph that (a) finishes a node,
    (b) then drives `apply_completion` into its `Err` path on a path that matters, and (c) is replayed
    from the journal. Pick whichever `Err` path is reachable (loop-budget exceeded is the named
    candidate) and say which one and why. Use the existing harness patterns in `crates/pii-graph/tests/loops.rs`
    and `crates/pii-graph/tests/resume.rs`.
R2. MEASURE, do not assert a belief. Report exactly one of two outcomes, with evidence:
    - **DIVERGENCE**: the replayed run's activated-edge set or final state differs from the live run's.
      Give both values verbatim (live vs replayed) and the minimal step where they part.
    - **NO DIVERGENCE**: the states are equal, and say WHY the `Err` path cannot matter here (what
      recovers it, and where).
    Either outcome must come from RUNNING the code, not from reading it: paste the test name, the run
    command, and its output.
R3. If DIVERGENCE is found, do NOT fix it. Keep the reproduction as a `#[test]` that documents the
    current behaviour (name it so it reads as a finding, e.g. `replay_loses_activation_on_loop_budget_err`),
    mark clearly in a comment that it pins CURRENT behaviour and is a defect record, and write the
    counter-example (the live path's edge set) in the comment. A fix is a separate spec.
R4. If NO DIVERGENCE is found, say so plainly and add NO test — the honest answer is "the discard is
    safe here, here is why", and the allow's comment stands as written.

## Exit Criteria
- [ ] `cargo test -p pii-graph` passes (whatever the outcome, the suite is green on the final tree)
- [ ] the measurement is recorded verbatim in `<scratch>/spec93/FINDING-6-MEASUREMENT.md`: the test name, the exact command, and the observed output for both the live and the replayed run
- [ ] the report states DIVERGENCE or NO DIVERGENCE explicitly in its first line, with the reason
- [ ] if DIVERGENCE: the reproduction test exists in `crates/pii-graph/tests/` and FAILS/FIXES nothing (it asserts the current, divergent behaviour and is labelled as a defect record); the report names what a fix spec would have to change
- [ ] if NO DIVERGENCE: no production or test file is changed by this spec, and the report says which mechanism makes the `Err` path harmless
- [ ] `bash quality-gate.sh` exits 0 and `cargo run -q -p xtask -- fresh-gate --worktree` prints `fresh-gate: PASS`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` exits 0 and `cargo fmt --all --check` exits 0
- [ ] the report lists every file touched (or states "none")

## Guardrails
- MEASUREMENT FIRST. Do NOT patch `replay`, `apply_completion`, or the `let _ =` discard to change
  behaviour.
- Do NOT weaken, ignore or delete an existing test.
- Do NOT invent a divergence: if the states match, that is the finding and it is a good outcome.
- Scratch under `<scratch>/` only; delete your own exports (one fresh-gate export is ~8G).
- Do NOT commit; leave the tree for the orchestrator.
