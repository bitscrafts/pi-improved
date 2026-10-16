# DEFECT 2026-10-16 — the boss watch stops growing its journal under load

**Found by** an accident with a purpose: a failing gate step (`test (lib/bins/tests)`) on a 32-core host, then a
deliberate CONTROL — the same test re-run with 32 CPU hogs pinned first, because a fix to a load-sensitive test
proven on an idle machine is a check that lies.

## The measured fact

```
panicked at crates/pii-server/tests/boss_watch.rs:244:9:
"the journal stopped growing: condition `journal len >= 6` (last journal len 5,
 no growth for 60.008407437s, absolute deadline 179.991483974s)"

test result: FAILED. 7 passed; 1 failed; 1 ignored; finished in 96.80s
```

The journal **reaches 5 entries and then stops for a full 60 seconds**. The test wants 6 — one more entry — to
observe the watch reach `detect` after a resume. So this is not "the test was slow": the watch stops making
progress, at a fixed point, under CPU contention.

## Two independent reproductions

1. **The gate's own run** (`fresh-gate-parallel-20261016T102802Z.log`): the same test, in a fresh export, with
   llama.cpp compiling in the same run — `failing tests: ["boss_watch_reaches_detect_after_resume"]`.
2. **The synthetic control** (`boss-watch-under-load-20261016T114846Z.log`): 32 spin loops pinned, then the test
   alone — same failure, and the stall bound reported it as a STALL rather than as elapsed time.

Two different loads, same point, same shape.

## Ruled out, with evidence

- **A wall-clock race in the test.** The first version asserted a deadline set for an idle machine. That was
  replaced with a stall bound (`no growth for the budget`, plus an absolute cap). The failure persisted and its
  message changed to name the stall — so the deadline was a *symptom*, not the cause. Recorded because the repair
  is what produced the diagnosis.
- **A missing toolchain.** `clang` and cmake/make/ninja were genuinely missing and genuinely made every earlier
  gate run die; installing them took the gate from 0 to 13 of 14 steps green. They are not this.
- **The host being saturated to the point of no progress.** Load 39.65 during the run, yet all other suites pass
  (7 of 8 in this very binary, and every other binary's) — the machine is busy, not wedged.

## Hypotheses, in the order they should be tested

1. **Blocking work inside an async task.** The watch advances through journal entries and stalls at a fixed entry.
   The classic shape: a synchronous store read (`store.read_journal`) or a lock held across an `.await` on the
   tokio runtime, starved when the box is busy. Cheap test: instrument the watch's step boundaries and see which
   boundary stops printing under load.
2. **A fixed internal timeout inside the watch's step.** A step that gives up after N seconds and neither retries
   nor records why would leave the journal exactly one entry short. Cheap test: grep the watch for timeouts and
   assert each one records its expiry as an event.
3. **A lost wakeup.** If the watch waits on a notification written *before* it subscribed, it will wait forever
   having done 5 of 6 entries. Cheap test: a counter of wakeups requested vs consumed.

## What a fix must look like here

- a test that reproduces **under load**, with a **control** that must still pass on an idle host;
- and, either way, the watch must **name which step blocked and for how long** — the same repair the gate needed
  when it could only say FAIL: a status an operator can act on.

## For a review (if one is called)

The narrow question, not a full sweep: *why does the boss watch stop at a fixed journal entry under CPU
contention, and which of the three hypotheses does the instrumentation eliminate?* Give it this file, the test,
the watch's source, and the two logs — and ask for the fix that makes the failure diagnosable by itself.

## 2026-10-16 18:25Z — the defect is cleared on HEAD, and the in-flight barrier reddens two length-waiting tests

Measured by the orchestrator on the tick that owns the plan queue. The working tree carried one uncommitted hunk
in `crates/pii-graph/src/sched/dispatch.rs` (a second lane's in-flight D3 work, session
`20261006_131058_33628f`, file mtime 17:24:09Z): a `flush()` placed between the `NodeStarted` append and
`run_caught`. It was **not touched and not committed** by this tick.

1. **HEAD is green for the whole file.** `git worktree add --detach /projects/_tmp/wt-d3check 6419a70`, then
   `CARGO_TARGET_DIR=/projects/pi-improved/target cargo test -p pii-server --test boss_watch` →
   `test result: ok. 9 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.36s`, and it
   includes both `boss_watch_reaches_detect_after_resume` and `boss_watch_gate_suspends_and_approve_resumes`.
   The stall does not reproduce on HEAD, and the file no longer pays a cold nested xtask build (0.36 s).

2. **The same command on the working tree (with the uncommitted barrier) → `7 passed; 2 failed`:**
   `a_timed_out_gate_is_inconclusive_never_detected` (`a timed-out gate must finish on \`inconclusive\`, got []`)
   and `boss_watch_timer_suspends_for_real` (`spawn_resume must genuinely re-enter dispatch`, `left: 1 right: 2`).
   Both fail alone under `--exact --test-threads=1`, so this is not test interference.

3. **Cause, read from the test:** both wait by *length*, not by event —
   `wait_for_journal_len(&engine, &run_id, before.len() + 1)` (`boss_watch.rs:359` and `:940`). Under the
   write-behind journal a `+1` becomes visible only at a flush, i.e. after the node finished. The barrier flushes
   `NodeStarted` *before* the node runs, so the wait returns one event early and the assertion then reads a journal
   in which `check` has not finished: `ports = []` for the timeout test, the first suspend only for the timer test.

4. **What the barrier therefore owes:** the two waits must be by event, not by length (wait for the `check` node's
   `NodeFinished`, or for the resumed `Suspended { reason: Timer }`), before the barrier can be committed. The
   barrier is what doc 064 R1 asks for; the tests' progress rule is the thing that has to move with it.

Queue state at that moment: **A22 (155) and A27 (160) stay parked.** The blocker both closures named —
`boss_watch_reaches_detect_after_resume` — is green on HEAD, but the working tree is red, so a `fresh-gate` on it
cannot be green either. Re-queue the two rows (resume the sessions already on disk, do not re-derive them) once
the D3 batch lands with its test fix and the tree is clean.

Cost of this check: the worktree shared the main target dir (`CARGO_TARGET_DIR=/projects/pi-improved/target`),
2m03s of build and negligible disk on a 96 %-full volume (25 GB free). The worktree and its log are left in place:
`/projects/_tmp/wt-d3check`, `_tmp/autonomy/logs/head-boss_watch.out`.
