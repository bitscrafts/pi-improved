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
