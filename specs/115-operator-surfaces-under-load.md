# Spec: 115 — the operator surfaces under load (spec 110's findings F1, F4, F5, F6)

**Project**: <repo>
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
- Scratch under `<scratch>/`, delete your own, no daemon left running. Do NOT commit.

## Orchestrator verification (2026-10-10) — the four defects, verified against the artefact
**F1 (the one that mattered) — read at the source, then mutated.** `engine/dispatch.rs:308-325`
(`run_header`) asks the BROKER first (`engine.runs.lock().contains_key(run_id)`) and only then the store, so
"registered, no header yet" is *accepted* and never races into NotFound. My mutation — disabling the in-flight
branch (`if false && in_flight`) — turns
`an_in_flight_run_is_accepted_and_an_unknown_id_is_not_found` **RED** (`run_get_states.rs:98`); restored → 2 pass.
**F4/F5/F6 — my own live daemon, not the report's word:**
```
pii_process_rss_kb 20708                                    ← RSS exposed
pii_store_bytes_per_run 0                                   ← growth is a number an operator can watch
# pii_run_latency_ms: percentiles are the upper bound of a 10%-wide bucket (never under-reported,
  at most 10% + 1 ms over), whole ms rounded up, durations past ~24 min saturate; max is exact
                                                            ← the resolution is stated, as the spec required
# NOTE the main store never shrinks on its own: ... reclaim interior free pages offline with `pii store compact`
sys status notes[] carries the SAME line                  ← one fact, two surfaces, no drift
RSS: metrics=21132 kB vs /proc=21136 kB                    ← 0.02% apart; the stated tolerance is 2%
daemons-left=0
```
**F5b — `pii store compact` is real, and it refuses when it must** (mine, on a fresh daemon):
```
daemon running:  error: invalid: store '…/store.redb' cannot be opened exclusively
                 (Database already open. Cannot acquire lock.); stop the daemon first
daemon stopped:  compacted store.redb 1056768 -> 36864 bytes   (×4: store, memory, approval, heal)
after:           all four stores at 36864 bytes            ← 1 056 768 → 36 864, no fake vacuum
```
**Tests**: `run_get_states` 2 pass · `status_latency` 1 pass · `store_compact` 2 pass.

**Notes worth keeping:** the schema files were regenerated (`pii-proto/schema/{run,event}.json`) because
`WireRunStatus` gained `Accepted` — a wire change, justified by the new state and reported rather than slipped in.
Honest limits kept: latency covers `spawn_run` only (suspends/resumes are not recorded); `run.list --status
accepted` returns empty because accepted runs are not stored; live run data is **not** reclaimable and no
retention policy exists for it — the runbook says so.

### The fresh-gate criterion CLOSED (2026-10-10, plain form, on 115's own HEAD)
`fresh-gate` (no `--worktree`, i.e. exactly HEAD `ea96f89`): **`quality-gate: ALL PASS` · `fresh-gate: PASS`**, exit 0,
70G free — and it ran **while spec 116's worker was already editing the tree**, which is safe precisely because the
plain form exports HEAD only. That is the corrected rule in action: plain form verifies a commit and is immune to a
concurrent writer; `--worktree` is not. **115: 8/8.**

