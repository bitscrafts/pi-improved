# INCIDENT 2026-10-14/15 — the gate convoy: five hours, zero items closed

**Impact**: no item closed after spec 152's first gate failure; load 29 → 97 on 32 cores; the laya decision head
starved (67+ min loading) and its fail-closed hook refused every state-changing tool call, mine included; an afternoon
of the operator's attention spent asking what was happening. Product code was never the problem.
**Status**: cause fully measured; fixes listed at the end with their true state.

## When (anchors)

- **Trigger** — spec 152's first `OUTCOME=gate-failed ITEM=A21`. Worker rc=0, its own tests passing, then the gate
  died compiling. Every retry after that added a worker and another cold 20-minute build. Five in total.
- **Amplifier** — my own experiment batch: seven workers between ~18:00 and 21:00Z, each building the workspace, plus
  a daemon build started and never collected. These turned a slow gate into a guaranteed timeout.
- **Convoy** — from ~20:30Z, `pgrep -c fresh-gate` = 17. All queued on cargo's single artifact lock. My own
  uncontended gate run then failed too: **RC=124**, killed at 1200 s inside clippy.

## Why (one cause, three conditions)

**Cause**: the gate cannot finish a COLD build inside its step budget — measured four times, killed at ~630 s while
compiling cranelift/wasmtime. Everything else follows.

1. Fresh worktrees are the design (correct, and staying) ⇒ every cycle builds from scratch.
2. The retry is not keyed to the outcome class: a TIMEOUT is treated as "not finished", so the loop re-dispatches
   instead of parking a non-retryable outcome.
3. Three schedulers can enter the same cycle (30-min cron tick, 5-min watchdog, 6-h agent lane) and the lock guards
   only ENTRY, not dispatch→verify→gate→commit ⇒ two cycles interleave and their builds queue behind each other.

**Why it was invisible**: the convoy starved the laya head; its hook then refused every call, so the symptom presented
as "laya is down" and hid a build problem for hours. The loop's own outputs (rc=0 workers, `gate-failed`) name no cause.

## Rules — so it does not happen again

- **R1 One entry point, for the whole cycle.** The lock spans dispatch→verify→gate→commit. The watchdog and the agent
  lane propose a next item; they never race one.
- **R2 One build at a time, and a build that resumes.** The gating build uses a dedicated persistent `CARGO_TARGET_DIR`
  (fresh worktree, warm compiler). A retry must be incremental, never a from-scratch recompile of the world.
- **R3 An outcome must wear its own name.** `gate-timeout` ≠ `gate-failed`. Retry only `gate-timeout`; park
  `gate-failed` for a person. A gate that says FAILED while the tree is fine is the same defect as a check that passes
  while the live path is broken.
- **R4 A retry budget per item, counted in outcomes, not attempts.** Five identical mechanical failures is a signal to
  stop and report, not to dispatch again.
- **R5 Never pattern-kill.** `pgrep -f <string>` matches the shell that carries that string: it took out my own caller
  three times in one day. Kill by the target's own `/proc/<pid>/cmdline`, excluding the caller and its parent
  (`scripts/autonomy/stop-cycle.sh` exists for this).
- **R6 Never edit a script a live process is executing.** Measured cost: a cycle died silently right after its gate
  passed. The guard (`pgrep` first, abort if a cycle runs) paid for itself twice tonight.
- **R7 Measure the machine before blaming a service.** The "laya problem" was our build load. One `ssh` reading load,
  RSS and elapsed time settles it in seconds; two hours of laya forensics did not.
- **R8 Evidence before plan.** No new plan until the previous one's first step is applied. Tonight's failure mode was
  correct diagnoses accumulating with zero executed fixes — the operator had to ask four times.

## Fixes and their true state

- WRITTEN: `scripts/autonomy/stop-cycle.sh --yes` (safe stop by argv).
- STAGED, NOT APPLIED: the persistent `CARGO_TARGET_DIR` for the gating build (one line in `cycle.sh:209`).
- NOT WRITTEN: the lock spanning the whole cycle (R1) · retry keyed to the outcome class (R3/R4).
- NOT TOUCHED, deliberately: the deploy path — it does not go through the loop at all.

## Detection that would have caught this

Spec 150's incident reader, over the loop's own `OUTCOME=` lines and gate logs, with a fingerprint for a repeated
`gate-timeout`/`gate-failed` on the same item: this storm is exactly 3+ identical outcomes inside one window. That
spec is already committed — it is the machine that opens an incident on its own retries, and tonight is its
justification. The operator-distance metric (H_pre / L / A) is the other half: tonight L was ~5 hours and H_pre was
large, and both were invisible.
