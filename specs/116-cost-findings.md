# Spec: 116 — the cost findings (spec 110's F2, F3, F8)

**Project**: /projects/pi-improved
**Phase**: Stage A2 of `057`, second row. Spec 110 measured a system nobody had ever loaded; three of its
findings are about what pii SPENDS, and two of them are still unexplained.

## What was measured (from `_docs/06-research/056-load-and-soak-2026-10-10.md`)
- **F2** — over 85 018 runs in 32.67 minutes RSS grew **60.5 MB -> 86.4 MB with NO plateau** (~152 KB per
  1 000 runs, ~155 B/run; ~25 MB/hour at the measured ceiling, and the audit log WAS included in the
  baseline config). Threads and fds were flat, so the growth is not thread/fd accumulation.
- **F3** — write amplification: ~**125-131 KB written per run to retain 2.5 KB** (~50x), roughly 3 GB per
  8 minutes at 44 runs/s. The journal, the store's write-behind and the events each write; the ratio is the
  diagnostic, not the total.
- **F8** — there is **no `NullObserver` switch and no way to disable the audit log**, so the specified A/B
  (observer attached vs detached) could not be run at all. Spec 110 refused to fake that row.

## Requirements
R1. **F2 — find it or bound it, with the bisect named.** Build a repeatable harness (a script under
    `scripts/` or an `xtask` subcommand, committed, re-runnable) that runs N thousand runs in a fresh store
    and samples RSS, then **bisect the run path** by disabling one component at a time: (a) the run table /
    engine state, (b) the journal write-behind, (c) the observer fan-out, (d) the trajectory sink (default
    OFF, so it is the control), (e) the client connection handling. For each variant report RSS vs runs and
    name which component accounts for the growth. **Then either fix the identified cause with a mutation-
    controlled test, or state precisely what was ruled out and what remains** — "no plateau" with a measured
    slope is a finding; a hand-wave is not.
R2. **F2 — the leak must be VISIBLE in the product**: a metric that would show it (RSS + runs + store bytes
    over time is the minimum) must exist **and** the runbook must say what slope is expected and what is not.
    A leak nobody can see is how this was missed for a week.
R3. **F3 — explain the ratio before changing it.** Break the ~50x down per writer (journal, store
    write-behind, events, audit log) with measured bytes per component per run — the sum must reconcile with
    the total within a stated tolerance. **If a reduction is available without weakening durability or the
    audit trail, take it** (and prove the reduction with before/after numbers); if not, say which component
    dominates and why it must stay.
R4. **F8 — make the A/B possible**: an observer switch that detaches the fan-out (an explicit
    `NullObserver`/`--observer off` path, config-gated, never the default) and an audit-off switch for
    measurement (**default ON and loud when off**, since the audit trail is a security surface). Then RUN the
    A/B spec 110 could not run and paste the result — including the honest "below resolution" wording if that
    is what it is.
R5. Every new surface: a test with a mutation control; every new name in `ALL`; every new switch documented in
    the runbook with its default and its risk.

## Exit Criteria
- [ ] the harness exists, is committed, and re-runs identically (paste the command and two runs' numbers)
- [ ] the bisect table with RSS-vs-runs per variant, and the component that accounts for the growth named
      (or an explicit "ruled out: a, b, c; remains: d" with the numbers that rule the others out)
- [ ] a fix with its mutation control, or the measured non-conclusion — pasted either way
- [ ] the RSS/leak metric exists and the runbook states the expected slope
- [ ] the write-amplification breakdown reconciles to the measured total within the stated tolerance, and any
      reduction is shown before/after
- [ ] the observer/audit switches exist, are OFF-by-default for measurement only, and the A/B is run and
      pasted (with "below resolution" if that is the answer)
- [ ] `bash quality-gate.sh` exits 0; `fresh-gate --worktree` prints `fresh-gate: PASS`; clippy/fmt exit 0
- [ ] the report lists files, config keys, event names, the tolerance, and what it did NOT measure

## Guardrails
- **Measurement is the deliverable.** Do not tune a threshold, cap or default to make a slope look flat. If
  you change a default, that change is the finding and it needs its own before/after numbers.
- A leak "fix" that hides the metric instead of the leak is a failure of this spec.
- Durability and the audit trail are not negotiable for speed: if a reduction would weaken either, it is a
  report line, not a commit.
- Keep the harness small enough to run on this host's disk (it is shared); delete your own exports.
- Do NOT commit; leave the tree for the orchestrator.

## Orchestrator verification (2026-10-10) — a real leak and a real 58.7% write reduction
**Report**: `_docs/06-research/058-cost-findings-2026-10-10.md` (tables, files, keys, tolerance, limits).

**F2 — the growth was mostly warm-up, with a REAL leak underneath, now fixed.** Ruled out by A/B inside noise:
observer fan-out, audit feed, connection handling (24 000 idle connections ≈ 4 B each). What remained was
`TrajectorySink` retaining a map entry per run forever AND scanning that map per event — a leak *and* a slowdown.
Fix: `trajectory.rs:206-216` releases a finished run's handle (`runs.remove(&run.0)`).
- **My own probe run** (`cargo test -p pii-observe --test rss_probe -- --ignored --nocapture`, 24.15 s):
  `runs 0/10k/…/70k → tracked_runs 0 at every step`, RSS 3 172 → 16 272 kB and **flat after 60k**
  (16 272 → 16 320 between 60k and 70k) — the retention is gone and the curve plateaus.
- **My mutation** disabling the release (`if false && release`) → `a_finished_run_is_released_by_the_sink`
  **FAILS** (`run 0 finished: nothing retained`); restored → green.
- **Residual honesty**: ~5-40 B/run remains unattributed in the store/runner path; the worker says it needs a heap
  profiler rather than a guess. Kept as a limit, not smoothed.

**F3 — the 50× was not the events: it was redb's commit granularity.** One commit costs **24.6 KB whatever it
carries**, and each run made four. Fixes: run ids are **reserved 32 at a time**, and `put_graph` no longer
re-commits a body already stored. Result: **124 150 → 51 236 B/run (−58.7%)**, throughput **~45 → 80-120 runs/s**;
in-process parts reconcile to the daemon total within **0.08%** (tolerance 5%).
**Behaviour change worth stating:** run ids can now **skip up to 31 after a restart** — strictly increasing, never
reused, and the whole suite plus the fresh tree pass with it.

**F8 — the A/B finally ran, and the honest answer is "below resolution"**: `--observer on|off`, `--audit on|off`
(and `[core] observer|audit`), both default ON and loud when turned off; every delta sits inside the ±1.5 ms/run
baseline spread. Spec 110 could not run this at all; now it can, and it says so.

**Tests I ran myself**: `a_finished_run_is_released` 1 pass · `log_wiring` 8 pass · `rss_gauge` 1 pass · the
sink-only probe 24.15 s pass.

**Limits carried:** the daemon was not run past 60 000 runs with trajectories ON (it still climbed ~270 B/run there;
the sink-only probe suggests the retention index filling toward its 50 000-file bound, not confirmed in the daemon);
RED-before-GREEN was observed only for the trajectory leak and the body dedup; the RSS gauge is not unit-tested
live above 4 000 runs; `cmd_daemon.rs` now sits exactly at the 450-line limit; scratch left at
`/projects/_tmp/leak-bisect` (~1 GB) per the no-delete rule.

**One unreported change, recorded here rather than glossed:** the commit also adds a `parking_lot` edge to
`pii-store` (`run_ids: parking_lot::Mutex<(u64, u64)>`, `lib.rs:50`) — the mutex guarding the batched run-id
reservation. `parking_lot` is already a workspace dependency used by six other crates, so it is a new EDGE and not
a new external crate, and it is justified by the fix; but the report did not mention it, and a dependency change
that nobody states is exactly what this lane asks to be stated. Accepted with the note.

### The fresh-gate criterion CLOSED (2026-10-10, plain form, on 116's own HEAD)
`fresh-gate` (exactly HEAD `f20203a`): **`quality-gate: ALL PASS` · `fresh-gate: PASS`**, exit 0, 58G free — with
spec 117's worker already editing the tree beside it. **116: 8/8.**

