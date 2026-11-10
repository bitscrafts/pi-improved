# Spec: 91 — finish the mutation verification of batches C/D (045 candidate 6)

**Project**: <repo>
**Phase**: 045 re-review, candidate 6 — the reviewer's delegated run was killed before it reported, so
these guards are still UNVERIFIED. This spec closes that gap with measurements.

## Context
The 045 report's §4 lists exactly what nobody has yet demonstrated. Two passes already exist and must
NOT be re-run (they are recorded in their specs): `specs/78-...md` (the seeded scope test goes red under
the pre-fix fail-open shape), `specs/79-...md` (the lag test goes red when the silent swallow is
restored), `specs/80-...md` (`is_heavy = false` turns all six heartbeat tests red and leaves the
light-method control green).

## Requirements — produce a mutation log, do not change production behaviour
For each item below: state BASELINE (green on HEAD) → MUTATE (the exact edit) → RESULT (red/green, with
the test name and assertion text), then RESTORE and confirm the tree is clean. A mutant that stays
GREEN is a FINDING, not a failure of this spec: record it and do not fix it here.

R1 (spec 77, journal flush): drain `pending` BEFORE `append_events` returns Ok (the pre-fix shape) →
    does `failed_flush_loses_no_events_and_seqs_stay_contiguous` go red? Then:
    - confirm `let_underscore_must_use` is genuinely enforced in pii-graph (enabled at source) and that
      no crate-level `#![allow(clippy::let_underscore_must_use)]` exists OUTSIDE pii-graph;
    - for each of the 3 named allows (render.rs, run/engine.rs, run_helpers.rs), say what it suppresses
      and whether the suppression is justified by the code at that site.
R2 (spec 78, resume fail-closed): in addition to the already-proven scope case, mutate (a) the graph
    lookup, (b) the budget/limits recovery, (c) the order of `register_run` (move it back BEFORE the
    recovery steps) — does each turn a test red? For (c), if NO test goes red, that is a finding.
R3 (spec 79, lag recovery): does the test create a GENUINE broadcast lag (not a synthetic
    `Lagged` injection)? State which it is. Then mutate the rescan away and confirm red.
R4 (spec 80, heavy methods off-worker): with `is_heavy` forced false (already proven), additionally drop
    ONE method from the heavy list (e.g. `MemoryPut`) and confirm the corresponding heartbeat test goes
    red. Confirm the light-method control is a real control (it must stay green under both mutations).
R5 (spec 82, store errors): `grep -c 'PiiError::Internal(' crates/pii-store/src/lib.rs` must be 0. Then
    (a) does `open_failure_renders_structured_store_op` pin the FULL message or only a prefix?
    (b) swap two ARGUMENTS in one `store_op(op, resource, detail)` call site (e.g. resource/op) — does
    any test go red? If not, that is a finding.

## Exit Criteria
- [ ] the mutation log exists at `<scratch>/spec91/MUTATION-LOG.md` with one row per mutation: file:line, the edit, the observed result, the test name + assertion, and RESTORED: yes
- [ ] every mutation is reverted and `git status --porcelain` shows ONLY files this spec was allowed to touch (report the list)
- [ ] `bash quality-gate.sh` exits 0 at the end (the tree is back to green)
- [ ] `cargo run -q -p xtask -- fresh-gate --worktree` prints `fresh-gate: PASS`
- [ ] `[ "$(grep -c 'PiiError::Internal(' crates/pii-store/src/lib.rs)" = 0 ]`
- [ ] `[ "$(grep -rn '#!\[allow(clippy::let_underscore_must_use)\]' crates --include='*.rs' | grep -v 'crates/pii-graph' | wc -l)" = 0 ]`
- [ ] the report lists EVERY mutant that stayed GREEN as a numbered finding with file:line

## Guardrails
- VERIFICATION ONLY. Do NOT fix anything, do NOT change a spec's implementation, do NOT "improve" a test
  to make a mutation red. Each mutation is applied, observed, reverted.
- Do NOT re-run the three passes already recorded (78 scope / 79 swallow / 80 is_heavy=false); cite them
  as previously verified instead.
- Scratch under `<scratch>/` only (the disk floor is enforced; one fresh-gate export ≈ 8G — delete
  your own scratch when done).
- Do NOT commit; leave the tree exactly as you found it.

## Result (2026-10-09) — the gap 045 could not close is now measured
Mutation log: `<scratch>/spec91/MUTATION-LOG.md` (copy committed to `_docs/06-research/046-`).
Tree left 0 dirty (verification-only respected). Both gates green.

**Guards with teeth (RED under mutation):** spec 77's flush test (drain-before-Ok → red, `left 0 right 3`);
spec 78's register-order (moved back before recovery → TWO tests red, including the no-run-registered
assertion); spec 79's rescan (removed from the `Lagged` arm → red) — and the log settles the reviewer's
doubt: **the lag is genuine** (5000 broadcasts into the real 4096-slot bus before the loop's first
`recv`, so tokio itself returns `RecvError::Lagged`), not a synthetic injection; spec 80's six methods,
one mutant per run, each → exactly one red heartbeat test with the control green; spec 82's `open` site
(swapped args → red).

**Findings — mutants that stayed GREEN (each becomes its own candidate):**
1. `runs.rs:~152-170` — a graph body that is unreadable/missing/unparsable falling back to an unlimited
   budget is undetected: there is no `get_graph` fault seam, and `resume_fail_closed_on_graph_lookup_fault`
   faults `get_run` (the header step) while asserting only `res.is_err()`.
2. `runs.rs:~167` — a resume that ignores the recovered limits (budget forced to `u64::MAX`) is undetected:
   `seed_real_run` (runs.rs:568) seeds limits that are already all `u64::MAX`.
3. `daemon/mod.rs:215` — `reactive_loop` never calling `lag::recover_finished` on a `Lagged` is undetected
   (the test at lag.rs:222 calls the helper directly). This is the wiring gap spec 88 admitted.
4. `pii-store/src/lib.rs:96,103,113,357` — swapping `op`/`resource` in a `store_op(…)` call site is
   undetected anywhere (workspace-wide for :357).
5. `pii-store/src/lib.rs:645` — `open_failure_renders_structured_store_op` pins only the prefix
   `store op open on `, not the resource or the detail (it catches :91 only because the op sits in the prefix).
6. **Observation, unmeasured** — `run_helpers.rs:125` discards the `Err` from `apply_completion` in
   replay; the "activation outcome irrelevant" justification may not hold for the loop-budget Err path,
   where the live run turns the error into a failure and replay does not. Needs a measurement, not a patch.

**Static checks confirmed:** `PiiError::Internal(` in `pii-store/src/lib.rs` = 0; `let_underscore_must_use`
is `#![warn]` at `pii-graph/src/lib.rs:12` (the gate's clippy `-D warnings` makes it an error) and **no
crate-level allow exists outside pii-graph**; the 3 item-level allows are each justified (String `write!`;
std-mpsc `send` whose receiver is dropped by design; replay `apply_completion` — see finding 6).
