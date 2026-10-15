# REGRESSION 2026-10-14 - the process worked last week; my changes made it unstable

**User's finding, verbatim**: "you understand the process worked last week! you did some modification that made the
system unstable! it needs to be documented." This record says WHICH modifications, and by what mechanism each turned a
working builder into a five-hour storm. Incident detail: INCIDENT-2026-10-14-the-gate-convoy.md.

## The working baseline (before this week)

The loop closed items: 133->142 then 145->151, all with green gates. The gate judged the repository's own tree, so its
target/ was ALREADY WARM: a gate run was minutes, a retry was nearly free, and an unbounded retry was harmless.

## The modifications, and how each destabilised the process

1. fresh-gate --worktree - the gate now judges a FRESH worktree overlay (introduced to fix a real defect: it had been
   judging HEAD, not the worker's tree, so a worker could report 4/4 while the gate was red).
   Mechanism: a fresh worktree means a COLD target/ on every run. A gate that cost minutes warm costs ~20 min cold and
   dies inside its step budget while compiling cranelift/wasmtime (measured four times, ~630 s). PRIMARY REGRESSION:
   it converted "retry is cheap" into "retry is a twenty-minute failure". The correctness intent is kept; the cost is
   removed by a dedicated PERSISTENT target for the gating build (applied 2026-10-15).
2. Two more entry points into the same cycle - a 5-minute detached watchdog and a 6-hour agent lane, both added by me
   for the never-stop requirement, on top of the 30-minute cron tick.
   Mechanism: the launcher exits immediately, so the lock it took was released the moment a cycle started. Three
   schedulers and no lock spanning the cycle = OVERLAPPING CYCLES (cycles=4 measured), whose builds queue on cargo's
   single artifact lock: 17 waiters, load 97. Fixed by making cycle.sh re-exec itself under flock, so the lock spans
   the cycle's whole life.
3. The unbounded retry was LATENT, and my change armed it. With a warm gate a retry cost nothing, so no cap was
   needed; with a cold gate the same code produced ELEVEN attempts on spec 152, a worker and a build each time.
   Fixed: three identical outcomes PARK the item with the reason recorded; never a fourth dispatch.
4. My own builds as amplifier: seven experiment workers (each building the workspace) plus a pii daemon build started
   and never collected, inside the same hours as the loop's gate runs - they took the cores the cold build needed.
5. control.sh pause does not stop an in-flight cycle (design choice when written): the operator pausing the loop still
   saw two cycles and their builds continue, so "pause" could not stop the burn.

## Rules this regression earns

- R-Measure: adopt an architectural change only with its cost measured on the real machine against the old cost.
  --worktree was right in intent and catastrophic in cost, and that cost was never measured first.
- R-Entry: one entry point per work loop, and the lock spans the WHOLE loop, not its entry. Extra schedulers may
  propose work; they may not race one.
- R-Cap: a retry cap is not optional because "retries are cheap" - that condition changes the moment any cost in the
  path changes, so the cap must exist BEFORE the cost changes.
- R-Quiet: an agent's own builds are indistinguishable from a load problem to the machine; the agent's builds are part
  of the change surface and must be counted and reported as such.

## True state now (2026-10-15)

Applied and verified: persistent gating target - lock spanning the whole cycle - three-outcome park cap -
stop-cycle.sh (safe interrupt by argv). Staged in DECISION-fresh-worktrees-and-a-resumable-gate-build.md: a timeout
must carry its OWN outcome (gate-timeout) rather than the same word as a broken tree. Not yet written: that outcome
class itself.
