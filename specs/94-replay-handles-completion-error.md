# Spec: 94 — replay handles a completion error the way the live path does (fixes spec 93's divergence)

**Project**: /projects/pi-improved
**Phase**: 045 follow-through. Spec 93 MEASURED a real divergence and left a defect-record test. This
spec fixes it, so a resumed/replayed run reaches the same state the live run did.

## Context — the measured defect
`crates/pii-graph/src/run_helpers.rs:125` discards the `Result` from `apply_completion` in `replay`
("activation outcome irrelevant"). It is NOT irrelevant on the loop-budget error path: the live path
(`crates/pii-graph/src/sched/dispatch.rs:368-376`) reacts to that `Err` by marking the node failed,
recording it in `failed`, and setting the run's failure — replay does none of that, so it leaves the
node's outbound edge `Pending` and `failure` unset.

Measured with `fix(loop_budget=1) -> gate` (spec 93, `_tmp/spec93/FINDING-6-MEASUREMENT.md`):
- LIVE ends `Failure("loop_budget exceeded for node 'fix'")`, marks the `fix`→`gate` edge Dead.
- REPLAYED ends `Failure("no progress: node 'gate' never became ready")`, edge still Pending.
- They part at journal seq 3 (the `fix` `NodeFinished`).
- Control: with `loop_budget=3` a later `retry` re-entry makes the divergence vanish — so the defect is
  specific to a budget-`Err` that is not followed by re-entry, and the CONTROL test must stay green.

**The fix direction is already validated by the orchestrator**: patching `replay` to mirror
`dispatch::apply`'s `Err` handling (`apply_failed_or_cancelled` + `failed.insert` + `failure = Some(e)`
+ skip the completion bookkeeping) makes the replayed terminal agree with the live one, and turns
spec 93's defect-record test RED as expected.

## Requirements
R1. In `replay`, handle the `Err` from `apply_completion` exactly as `dispatch::apply` does. Keep the
    success path's current behaviour byte-for-byte (the `let _ =` stays legal only if the `Err` is
    handled; the item-level `#[allow(clippy::let_underscore_must_use)]` is removed if the call is no
    longer discarded — keep the allow only where a discard genuinely remains).
R2. State the ordering question explicitly and match the LIVE order: the live path records the value on
    the blackboard BEFORE `apply_completion`; if replay's current order (record after) can differ from
    live on the `Err` path, align it and say so in the report. If the order cannot matter, say why.
R3. Update spec 93's defect-record test: `replay_loses_activation_on_loop_budget_err` becomes a
    positive assertion (live == replayed) with a name that reflects the fix (e.g.
    `replay_matches_live_on_loop_budget_err`), and its comment stops calling the behaviour a defect.
    Do NOT delete the control test.
R4. Re-run the measurement: the report must show the SAME experiment (`fix(loop_budget=1) -> gate`) now
    producing equal terminals, with both values pasted.

## Exit Criteria
- [ ] `cargo test -p pii-graph` passes, including the rewritten test and the untouched control
- [ ] the rewritten test would catch a regression: **reverting this spec's `replay` change makes it RED** (record the verbatim assertion, then restore)
- [ ] `cargo test -p pii-server`, `cargo test -p pii-store` pass (no blast radius)
- [ ] `bash quality-gate.sh` exits 0 and `cargo run -q -p xtask -- fresh-gate --worktree` prints `fresh-gate: PASS`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` exits 0 and `cargo fmt --all --check` exits 0
- [ ] `grep -c 'let _ = crate::sched::state::apply_completion' crates/pii-graph/src/run_helpers.rs` is 0
- [ ] the report lists every file touched, and states whether the LIVE path changed by a single byte (it must not: say so explicitly)
- [ ] `/projects/_tmp/spec94/REMEASURE.md` holds the before/after terminals from the same experiment

## Guardrails
- The LIVE path (`sched/dispatch.rs`) must not change behaviour. This is replay alignment only.
- Do NOT change the graph semantics, the `Err` values, or the budget logic to make the terminals agree.
- Do NOT weaken the control test, and do NOT remove the `Err` handling from `dispatch::apply`.
- Scratch under `/projects/_tmp/`; delete your own exports (one fresh-gate export is ~8G).
- Do NOT commit; leave the tree for the orchestrator.

## Result (2026-10-09) — the divergence is gone, and the test now pins the fix
- `replay` mirrors `dispatch::apply` on `Err`: `apply_failed_or_cancelled` + `failed.insert` + `failure = Some(e)`
  + skip the completion bookkeeping; the `let _ =` and its `#[allow]` are gone.
- R2 answered by CHANGING the order: replay now records on the blackboard BEFORE `apply_completion`, as the
  live path does (harmless on the success path — `apply_completion` never reads the blackboard).
- Beyond the spec, flagged by the worker: the `Err` path still calls `suspended.remove(node)`, as replay's
  other failure branches do; the live path has no equivalent because a node that just finished is not
  suspended. Kept and recorded.
- **REMEASURE** (`_tmp/spec94/REMEASURE.md`): before — live `loop_budget exceeded for node 'fix'`,
  replayed `no progress: node 'gate' never became ready`; after — both `Failure { error: "loop_budget
  exceeded for node 'fix'" }`.
- **Reverse mutation reproduced by the ORCHESTRATOR**: restoring HEAD's discard turns
  `replay_matches_live_on_loop_budget_err` RED with exactly
  `left: loop_budget exceeded … / right: no progress: node 'gate' never became ready`, while the control
  stays green. So the rewritten test pins the fix rather than describing it.
- `sched/dispatch.rs` unchanged (live path untouched); only the two files above moved.
- Both gates green: `quality-gate.sh` ALL PASS · `fresh-gate --worktree` PASS.
