# DECISION — fresh worktrees stay; the BUILD must be resumable (2026-10-14)

**User directive, verbatim**: *"we already decided on fresh worktrees! it need to be resumable!"*
This settles the design, and it overrides my first proposal: a shared target dir is not a way to avoid fresh
worktrees — fresh worktrees are the design. What follows from it is the actual defect and the actual fix.

## The design (unchanged)

Each cycle gates a **fresh worktree** of HEAD overlaid with the worker's tree (`fresh-gate --worktree`). Isolation is
the point: the gate judges the worker's artefact, never a tree that earlier cycles have warmed or dirtied. That stays.

## The defect this exposes

A fresh worktree means a **cold `target/`**: every gate run recompiles the whole workspace (wasmtime + cranelift +
hundreds of crates). Measured 2026-10-14: `HEARTBEAT step=build elapsed=630s` and still compiling, then
`fresh-gate FAILED` — **no error, no failed test, no clippy finding. The gate was killed while compiling.**

Consequences measured today, all from this one cause:
- 152 failed its gate five times with its own tests passing each time;
- "progress almost zero" — every retry pays the cold build again;
- load 8 → 29 → 40 on 32 cores — the repeated cold builds *are* the load;
- that load starves the laya decision head (11-minute load), whose hook then times out at 15 s and refuses calls →
  the PASS/BLOCK oscillation.

## The fix, in the user's terms — resumable, not warm-by-avoidance

1. **Resumable build**: the gate's build uses a **persistent `CARGO_TARGET_DIR`** dedicated to gating (e.g.
   `_tmp/gate-target/`), passed explicitly into the `fresh-gate` invocation. The worktree is still fresh and still
   thrown away; the *compilation cache* survives, so a re-run after a timeout resumes incrementally instead of
   starting from zero. Freshness of the artefact and warmth of the compiler are independent, and only the second was
   ever a problem.
2. **A timeout is not a failure** — it gets its own outcome (`gate-timeout`), exactly as the account window got
   `window-closed`. `FAILED` must mean "the tree is wrong"; a killed build must say "not measured", because a gate
   that reports FAILED while the tree is fine is the "a check that passes while the live path is broken" rule inverted:
   a check that fails while the work is good.
3. **Resumable sessions stay as they are** (already true): the dispatcher records the session id per item and the
   cycle resumes it with a corrective brief rather than starting fresh — `session=549ed093` for 152's last worker.

## Acceptance for the fix

- a gate run that times out mid-build, re-run, completes **without recompiling from scratch** (the second run's build
  step is orders of magnitude shorter — the before/after pair is the evidence);
- the timeout case records `gate-timeout` and the item is **retried**, not parked as a code failure;
- the existing five failures of 152 re-gate green when the tree is green, i.e. the item closes;
- load during a retry cycle falls, and the laya head's load time returns to its idle figure — the measurement that
  proves the whole chain was one cause.
