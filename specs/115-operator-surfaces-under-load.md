# Spec: 115 — the operator surfaces under load (spec 110's findings F1, F4, F5, F6)

**Project**: /projects/pi-improved
**Phase**: Stage A2 of `057` — the defects the first measurement found. **A surface that lies, or grows
without bound, makes every later measurement untrustworthy**, which is why these come before the vision stages.

## What was measured (do not re-derive; from `_docs/06-research/056-load-and-soak-2026-10-10.md`)
- **F1** — 8 connections × 300 s produced **12 452 runs and 128 054 `run.get` NotFound replies**: a client
  polling a KNOWN run cannot tell *"not yet materialised"* from *"no such id"*, and a run that failed
  validation at start stays NotFound forever.
- **F4** — trajectories have per-run caps but **no aggregate bound**: 85 019 files / 196 MB in 33 minutes,
  i.e. ~160k inodes and ~390 MB per hour at the measured ceiling. Nothing prunes, and nothing refuses.
- **F5** — the store never shrinks: 1.06 MB → 216.6 MB over 85 018 runs (2.5 KB/run). `pii_store_bytes` is the
  only visibility and it says nothing about the trend.
- **F6** — `pii metrics` exposes no latency percentiles and no RSS, so under load the numbers an operator
  needs are invisible; spec 110 had to measure them from outside the process.

## Requirements
R1. **F1 — "not yet" must be distinguishable from "unknown", for a KNOWN run.** `run.get` (and whatever the
    CLI/TUI poll) must answer with a state that names the difference: an in-flight/accepted-but-not-yet-
    materialised run is reported as such, an unknown id as unknown. Drive it with the broker rather than the
    store so the fix does not depend on when the journal flushes. **A run whose start was refused must reach
    a terminal state (or a named refusal) — never an id that polls NotFound forever** (that is spec 106's rule
    applied to the start path).
R2. **F4 — trajectory retention is bounded, and the bound says so.** Add an aggregate policy (total bytes
    and/or file count, config keys) with oldest-first pruning; emit a name-d event when the policy prunes or
    refuses; keep the per-run caps. Prove with a tiny budget that the sink prunes instead of growing, and that
    a pruned trajectory is still readable for the runs that remain. Never silently drop the CURRENT run.
R3. **F5 — store growth is visible, and reclamation is either real or honestly absent.**
    a. `pii metrics` exposes what an operator needs to see the trend (bytes and runs, or bytes-per-run
       directly), so growth is a number, not a surprise.
    b. `pii store compact` (offline, refuses while the daemon holds the store) that rewrites the store into a
       smaller file and swaps it in atomically — **or**, if you conclude redb cannot do this safely here, say
       so in the runbook with the measurement that shows why, and make `a` carry the whole load. Do not fake a
       vacuum: a command that claims to shrink and does not is worse than no command.
    c. The status/metrics path must state that the store never shrinks on its own (one line an operator reads
       before a disk fills).
R4. **F6 — the loaded-system numbers come from the product, not from an outside harness.** `pii metrics`
    exposes, as instrumented counters (no new dependency): run-lifecycle latency percentiles (or a bounded
    reservoir/histogram with a stated resolution) broken down by outcome, and the process RSS. The numbers must
    agree with an outside measurement of the same workload within a stated tolerance — paste both.
R5. **Discipline for every new surface**: a test per requirement with a **mutation control** that turns it red;
    every new event name registered in `ALL`; a new name emitted NOWHERE is the spec-109 defect and fails the
    review.
R6. **Docs**: the runbook gains the retention policy, the growth expectation with its measured rate, the new
    metrics, and the compaction/`never shrinks` statement matching what the code actually does.

## Exit Criteria
- [ ] F1: a poll on a KNOWN in-flight run answers distinctly from an unknown id (paste both, with the CLI
      wording); a refused start's id reaches a terminal/named state (paste)
- [ ] F1 control: the mutation that makes "in flight" answer like "unknown" turns the test red
- [ ] F4: with a tiny budget the trajectory dir stays under it, the pruning event is emitted, the current
      run's trajectory is intact (paste `ls | wc -l` before/after and the event)
- [ ] F5a: metrics carry the growth numbers (paste); F5b: `pii store compact` either shrinks a real store
      (paste bytes before/after) or the runbook states why not, with the measurement
- [ ] F6: `pii metrics` shows the percentiles and RSS; the same workload measured outside agrees within the
      stated tolerance (paste both, name the tolerance)
- [ ] `grep` each new name const appears in `names.rs`'s `ALL` (paste)
- [ ] `bash quality-gate.sh` exits 0; `fresh-gate --worktree` prints `fresh-gate: PASS`; clippy/fmt exit 0
- [ ] the report lists files, config keys, event names, and the tolerance it used

## Guardrails
- **No lying counters.** A percentile computed from a bounded reservoir must say its resolution and what it
  cannot see; a "compaction" that does not shrink is a lie — implement it or state its absence.
- **Never silently drop the current run's data.** Pruning may remove old trajectories only.
- Do not raise any cap to make a test pass; a cap that has to move is a finding.
- Do not add a dependency for latency/RSS measurement; the counters are in-process.
- Scratch under `/projects/_tmp/`, delete your own, no daemon left running. Do NOT commit.
