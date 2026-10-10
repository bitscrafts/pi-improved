# Spec: 119 — the suite must be load-stable, or a red means nothing

**Project**: /projects/pi-improved
**Phase**: **Stage A4** of `057`. The gate's whole value is that **red means broken**. Spec 118 could not finish its
fresh-gate criterion because the suite is not load-stable: inside `fresh-gate`, `cargo test --workspace` failed on
**different tests on consecutive runs**.

## What was measured (2026-10-10)
- Run 1 inside a fresh export failed `resume_never_reinvokes_completed_node`; run 2 failed
  `diamond_race_cancels_loser` and `diamond_race_cancels_multi_node_arm`. All of them pass when run alone and pass
  in the working tree — a load-dependent, non-deterministic failure.
- The resume test was already root-caused once today: it cancelled a run after a **fixed 300 ms sleep** instead of
  waiting for a condition, and the journal it polled is **write-behind** (one flush carried `NodeStarted(b)` and
  `NodeFinished(b)` of the same tick together at 6.1 s). It was fixed to wait on the node's own log write — 24 runs
  under 16 CPU hogs passed — **and it still failed once inside a later gate run.** One reproduction is not proof;
  hence this spec is about the CLASS, not that one test.
- The two diamond tests are cancellation-race tests (`crates/pii-graph/tests/diamond.rs:269,304`) — a natural
  hiding place for wall-clock assumptions.

## Requirements
R1. **A committed, re-runnable harness** (`scripts/` or an `xtask` subcommand — HARD RULE 0d: no ad-hoc inline
    dispatch) that runs the gate's own command, `cargo test --workspace`, under a **stated** CPU load for N rounds
    and prints **per-test failure counts across rounds**. State the load and the round count in the report.
R2. **Name every timing-sensitive test with its measured failure rate** (per-round table pasted; a test that never
    failed is still listed, marked "0 of N").
R3. **For each, name the assumption in one line.** Candidate families: a fixed `sleep` where a condition is meant;
    a timeout used as synchronisation; a join without a deadline; a per-test port that parallel binaries can
    collide on; an ordering assumption between writer and reader; waiting on a **write-behind** store.
R4. **Replace it with the honest form** — bounded condition wait, injected clock/scheduler, or explicit
    serialisation of the contended resource — **without weakening the assertion**: for each fixed test provide a
    mutation that turns it red (re-enable the re-invocation, drop the cancellation, break the ordering) and paste
    the red.
R5. **Prove stability at gate load**: >= 10 rounds of `cargo test --workspace` under the stated load with **zero**
    failures, plus each fixed test green when run alone. Paste both.
R6. **Write the rule where it will load again** — a `DO-NOT-REPEAT` §1 row: *"a test that needs a duration to be
    correct is a defect: wait on a condition, inject a clock, or serialise the resource"*, citing this spec and the
    journal measurement.

## Exit Criteria
- [ ] the harness exists, is committed, and re-runs identically (paste the command twice with its counts)
- [ ] the per-test failure table across rounds (paste), tests that never failed included
- [ ] for every test that failed at least once: the assumption in one line + the fix (paste the diff hunk)
- [ ] a mutation per fixed test that turns it RED (paste), each restored
- [ ] >= 10 rounds of `cargo test --workspace` under load with 0 failures (paste), and the fast path green
- [ ] the `DO-NOT-REPEAT` row written (paste it)
- [ ] `bash quality-gate.sh` exits 0 (13 steps); plain `fresh-gate` prints `fresh-gate: PASS`; clippy/fmt exit 0
- [ ] the report lists files, the load used, the rounds, and what it did NOT reproduce

## Guardrails
- **Do not make the gate green by removing or skipping a test.** An `#[ignore]` requires an explicit reason and a
  filed follow-up spec; deleting an assertion is a fail.
- Do not lower a timeout, widen a tolerance, or serialise the whole suite to hide a race — state the cost of any
  serialisation you add (the suite's parallelism is real and the gate's wall clock matters).
- No test may depend on a `/projects/_tmp/` path existing, on wall-clock resolution, or on another test having run
  first.
- Scratch under `/projects/_tmp/`, delete your own, no daemon or broker left running. Do NOT commit.
