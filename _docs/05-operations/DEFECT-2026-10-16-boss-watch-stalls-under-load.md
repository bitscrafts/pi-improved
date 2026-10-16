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
