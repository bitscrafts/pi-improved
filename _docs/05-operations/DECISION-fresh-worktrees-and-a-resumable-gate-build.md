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

## Resuming a failed gate — the command, not the folklore

The gate keeps two things on failure, and they are the resume: the **export that failed** (`export kept at …` in the
error) and the **persistent target** (`$FRESH_GATE_TARGET`), so a re-run resumes compilation instead of rebuilding from
zero. `quality-gate.sh` takes its step list from `$GATE_STEPS` (`gate-steps.txt` by default), which means one step can
be run on its own.

So a failure is resumed like this, and only the failed step pays:

```bash
# 1. the gate's own error names them: "failed steps: [\"fmt\", \"clippy\"]; ... export kept at <dir>"
EXP=/projects/_tmp/fresh-gate-<pid>

# 2. the failed steps, as a step file (labels exactly as gate-steps.txt spells them)
grep -E '^(fmt|clippy|test \(lib/bins/tests\)|lint-scratch)\|' gate-steps.txt > /tmp/resume-steps.txt

# 3. fix, then resume IN THE EXPORT — the source that actually failed
cp -f crates/pii-cli/tests/whatever.rs "$EXP/crates/pii-cli/tests/"
cd "$EXP" && FRESH_GATE_TARGET=… TEST_GATE_TARGET=… GATE_STEPS=/tmp/resume-steps.txt bash quality-gate.sh
```

When the resumed steps are green, run the whole gate once — that is the verdict a deploy card may quote. Resuming is
for the loop; the card needs the whole thing.

**Why this is written here.** Measured 2026-11-02: a gate failed on `fmt`/`clippy`/`test`/`lint-scratch`, and the
answer to "and there is no resume procedure?" was that one existed and was in nobody's notes — so every retry paid a
full export and a full build for one broken semicolon.

## The disk is part of the harness — measured the hard way, 2026-11-02

A full hour was spent diagnosing "flaky tests" that were not flaky. The shape of the mistake is worth recording because
it is the exact rule this file already states, applied to the tooling instead of the code:

```
/tmp is on the ROOT volume (overlay, 288 G). Gate targets and clone targets were pointed there:
  /tmp/clone-gate            32 G
  /tmp/wt-target             3.5 G
  /tmp/base-target           0.9 G
plus one gate export on /projects: 29 G
→ / at 100 %, 0 bytes available
→ cargo: "failed to write full.rmeta: No space left on device"
→ test daemons: could not create their stores, so `until(...)` spun to its budget
→ five tests "failed" at every revision, including the one already in production
```

With 36 G free, the same two test files at the same revision passed in 10.3 s and 0.08 s. **Nothing was wrong with
the tests.**

Rules that follow:

- **`$FRESH_GATE_TARGET` and every clone target belong on the same large volume as the repo** (`/projects`), never in
  `/tmp`. The gate's own floor checks the workspace volume, so a full root filesystem passes every floor and fails
  every build.
- **Before believing a test failure that repeats at every revision, run `df -h /`.** A test that waits and times out is
  the signature of a full disk, not of a race.
- **Clean up what you create**: a gate export is ~29 G and a warm target ~32 G. Three of those end the afternoon.
