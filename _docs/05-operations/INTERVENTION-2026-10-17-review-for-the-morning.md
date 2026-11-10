# Intervention 2026-10-17 — what broke, what is fixed, what is queued

Read this first in the morning. Everything here is measured; nothing is a promise.

## THE DEPLOY (the one thing that never was in doubt)
`pii` is installed and running on the host as a systemd user unit: revision **0e34719dce0d** (the revision the deploy card
approved), `channels ['discord']` live, `channel_posts 1`, `channel_post_refusals 0`, and `kill -9` returns it in **8
seconds** with a new PID. The card was rendered with five fields, approved, and rehearsed: the rollback fails safe with no
restore source and restores otherwise (`deploy/rollback-pii.sh`, rehearsed in a copy prefix, live install untouched).
The 14/14 gate verdict on that revision is `_tmp/autonomy/logs/gate-run3-20261017T020154Z.log`.

## WHAT WAS BROKEN, IN THE ORDER IT WAS FOUND (each with its number)
1. **The gate rebuilt the whole workspace from scratch on every run.** Not one of its six build steps used the warm cache —
   `FRESH_GATE_TARGET` was used by nothing, and each step compiled into a directory created empty for that run. One rustc
   took 12 minutes; three linkers ran at once; load hit **88 on 32 cores**. FIXED: every step now points at
   `_tmp/gate-target/<step>` (commits `1759122`, `dbf3bf2`).
2. **`clippy (llamacpp)` compiled llama.cpp from source — 720 seconds measured tonight — for a code path the product never
   uses.** pii reaches models over HTTP (`llamacpp-server:`), and `llama-cpp-2` is an optional feature that is off in every
   other step. QUEUED as **A36 / spec 170**.
3. **Parallelism was uncapped, then capped wrong.** `CMAKE_BUILD_PARALLEL_LEVEL=$(nproc)` (added by me the previous night)
   let concurrent builds each claim all 32 cores. FIXED: `CARGO_BUILD_JOBS=8` and `CMAKE_BUILD_PARALLEL_LEVEL=8` in
   `scripts/autonomy/env.sh` (`084b011`). Measured effect: load fell from 88 to 9-35.
4. **The gate died five times with the bare message `git archive failed`** and parked two items at 5 and 6 attempts whose
   note read "the same mechanical failure repeated". FIXED: the export now reports git's stderr, rc, root and dir
   (`19024ce`).
5. **A failed item at the head of the queue freezes everything behind it, silently.** A22 failed at the gate, was set to
   `needs-review`, the tick refused to dispatch it, and no worker ran for 35+ minutes while every watcher said "fine". The
   loop's notice speaks ONCE per unique failure (`say_once`) and never repeats. QUEUED as **A39 / spec 169** (skip a parked
   head) and, deeper, the tick must branch on the QUEUE's state, not on the previous cycle's verdict.
6. **Nothing told anyone.** FIXED as an instrument: cron `0e576bca30e6` "pii park + silence alarm", every 20 minutes,
   delivered to Discord, printing one line when an item is parked or when there is pending work with no worker and no
   compiler for 45+ minutes; silent otherwise.

## THE WORK THAT LANDED TONIGHT
· `300b810` the reader sees the channel refuse: 1,425 identical `channel.poll.failed` events become one incident
  (`refusal-repeated:channel.poll.failed`, latency 4 s) on a copy of the daemon's own log. Before: zero incidents.
· `a57d56e` the review becomes queue rows: `plan_import.py`, 17 tests, invoked by the tick — built by claude+sonnet from
  spec 167 on opus doc 066's owner decision.
· `0055ff1` the boss-watch test stops building a second xtask: 96.8 s → 0.41 s under 32 hogs.
· `3c6b11a` the in-flight barrier, time-scoped (064's D3), its `write_amp` criterion green.
· `19024ce`, `1759122`, `dbf3bf2`, `084b011`, `daf185f`, `0f4b38e` and the DO-NOT-REPEAT entry.

## QUEUED FOR THE LOOP (in order)
A22 155 · A27 160 · A28 161 · A29 162 · A30 163 · A31 164 · A32 165 · A33 166 · A35 168 · A36 170 · A37 172 · A38 173 ·
A39 169 · A40 171.

## THE HONEST PARTS
· My "it is running" reports twice described an alive compiler, not a model being called. A live process is not progress;
  a frozen usage counter with no `claude` process is the truth.
· The `FRESH_GATE_TARGET` warm cache I claimed to have added was used by nothing on that path; I reported a cache as warm
  that no step read. The fix landed only after the correction.
· The evening cost the user roughly **1 h 40 m** of console stillness, in two cold gates, for work that option A40
  (two-tier gate) removes permanently.

## THE MEASUREMENT THAT MATTERS — added 07:12Z, from the cycle that ran while the user slept

    [05:26:35Z] cycle starts, item=A22 (155)
    [05:27:16Z] worker rc=0 session=048de4cf-…            ← THE WORKER TOOK 41 SECONDS
    [05:27:16Z] verify: 4 criteria, each rc=0             ← the spec's own checks all passed
    [05:27:17Z] verify: bash quality-gate.sh
    [07:12:06Z] rc!=0 → OUTCOME=verify-failed ITEM=A22    ← THE GATE TOOK 1 HOUR 45 MINUTES

  ⇒ 99.6% of a cycle is the gate. The model work is 41 seconds. Every hour of tonight's silence was spent compiling,
    not thinking, and the warm-cache change (1759122) did not cure it — because the `build` and `test` cache directories
    were brand new and had to be filled, and because `clippy (llamacpp)` alone is 720 s of it.

  AND IT STILL FAILED. So the loop parked A22 again, and the queue behind it stops until a person clears it — the
  deadlock A39/spec 169 describes, now with a 1 h 45 m price tag attached to every attempt.

## WHAT THIS CHANGES
  The overnight job (3efcc28a27c1) keeps cycling every 30 minutes, but each cycle is hostage to a 1 h 45 m gate, so expect
  FEWER results by morning than anyone would like — that is the honest forecast, not a promise.
  The cure is not another cache: it is A40/spec 171 (routine gate in the repo, warm, per item; the fresh export only
  before a deploy) plus A36/spec 170 (drop the 720 s llamacpp step). Both are queued, both are small, and until they land
  the loop's throughput is one item per two hours at best.
