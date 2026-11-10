# 183 — the lagged cron chain still starts the next firing

## Overview
MEASURED 2026-10-18 by the orchestrator on a clean tree (`git status --porcelain` empty, `dbe600a`):

```
cargo test -p pii-server --lib    -> 127 passed; 2 failed; 1 ignored   (rc=101)
  daemon::lag::tests::lagged_run_finished_still_starts_next_cron_firing        lag.rs:231
      assertion `left == right` failed: next firing must have started: {}      left: 0  right: 1
  daemon::lag::tests::reactive_loop_lag_rearms_cron_chain_for_lost_run_finished lag.rs:309
```

**Introduced by `03e729a`**, the commit that carries spec 179's real fix (a cycle committed it as
"leftover from the previous item"). It changed `crates/pii-server/src/daemon/mod.rs` so a cron run stays tracked
until it truly ends:

```
-    let cron_name = ctx.cron_runs.lock().remove(run_id);
+    // re-arming here started a new run every CRON_MIN_REARM. Stay tracked until it truly ends.
+    let cron_name = if parked {
+        ctx.cron_runs.lock().remove(run_id)
```

That is right for the defect 179 measured (91 → 516 runs in four minutes, one run per re-arm) and wrong for the
lag path: after a **lagged** `run_finished` the chain must still start the next **scheduled occurrence**, and the
two tests above are exactly that contract. Both intents can hold at once — the run must stop being re-armed
without the next timer occurrence being swallowed — so this is a fix in the cron/lag bookkeeping, not a choice
between 179 and its neighbours.

**Why it shipped red:** the routine gate the loop runs per item is `gate-steps-quick.txt` (**fmt, clippy, build**
only). `test (lib/bins/tests)` is a step of `gate-steps.txt`, the commit and release boundary. So 179's item
closed "gate green" with the workspace suite red, and the loop's own gate never ran the tests that would have
said so.

**Project**: <repo>

## Requirements
1. `daemon::lag::tests::lagged_run_finished_still_starts_next_cron_firing` and
   `daemon::lag::tests::reactive_loop_lag_rearms_cron_chain_for_lost_run_finished` pass **as written**. They are
   the contract, not the defect: do not edit them to match the new bookkeeping.
2. Spec 179's own proof keeps holding: `crates/pii-server/tests/timer_no_refire.rs` still shows one run per
   occurrence, including its six-second control (six times the 1 s minimum re-arm).
3. No regression in the rest of the crate: `cargo test -p pii-server --lib` reports **at least 129 passing**
   (127 + the 2 above), 0 failed.

## TDD Contract
- **RED (measured, reproduce it first):** `cargo test -p pii-server --lib` → the two failures above with
  `left: 0  right: 1`. Record the counts before touching anything.
- **GREEN:** the same command reports 0 failed and at least 129 passed, and `timer_no_refire` stays green in the
  same tree.
- A fix that makes the lag tests pass by re-arming every `CRON_MIN_REARM` again is not green: `timer_no_refire`'s
  six-second window must still show one run, not six.

## Exit Criteria
- [ ] `cargo test -p pii-server --lib`
- [ ] `cargo test -p pii-server --test timer_no_refire`

## Guardrails
- Do NOT delete, rename or weaken the two lag tests, and do NOT lower the run count they assert.
- Do NOT revert 179: the measured 516-runs-in-four-minutes defect must stay closed.
- Do NOT raise `CRON_MIN_REARM` (or any interval) to make the loop disappear — the subject is what is tracked
  across a finish, not how often a re-arm is attempted.
- Do NOT touch `specs/`.
- If the two contracts genuinely cannot hold in the current data model (the parked/ended distinction has no place
  to live), **STOP** and report the shape you would need instead of picking one side quietly.

## Closure
(to be filled by the orchestrator: the two criteria run by hand, the counts before/after, and a mutation that
reddens `timer_no_refire` to prove 179's guard still bites.)
