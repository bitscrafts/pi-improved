# 065: Round 2 on 064, and the production-ready question (Opus, 2026-10-16)

This is a read-only review. The only repo file written is this one, and nothing was committed. Scratch work is in
`/projects/_tmp/rev065-QS7b/` (copies of the live daemon logs, 35 M). Evidence labels:
- **C-run**: I ran it today.
- **C-read**: I read the lines.
- **asserted**: from the brief or a commit message.

_Written incrementally._

## 0. What the brief says is done, verified

| item | verified | label |
|---|---|---|
| D1 `0055ff1` | It exists and touches only `tests/boss_watch.rs` (+41/−14). `c0b3601` records `9 passed … finished in 0.36s` on a worktree at `6419a70`. I did not re-run it. | C-run (stat), asserted (timing) |
| D2 `0737c8a` + `946d569` + `6419a70` | They exist. `CargoXtaskRunner` now sets `timed_out: out.timed_out` (`heal.rs`, `run_gate`). The port is declared in the second commit, and the proof test (+98 lines) is in the third. | C-read |
| D3 reverted | The copy is at `_tmp/dispatch.rs.my-edit-1792182709` (10-16 20:31). The tree is clean (`git status` empty). The budget is `per_run < 220` hundredths of a commit for a one-node `transform` run (`pii-graph/tests/write_amp.rs:85`). A flush on every `NodeStarted` made it 304, i.e. **+1 commit per node**. | C-run, C-read |
| `8dfea30` | The two tests wait on events (+65/−5 in `boss_watch.rs`). | C-run (stat) |
| "the gate completes at 13 of 14" | **The newest full-gate log on disk is 10-16 11:16** (`fresh-gate-parallel-20261016T102802Z.log`), which FAILED on `boss_watch_reaches_detect_after_resume`. That run is from before D1. **No full gate on disk has run on a HEAD containing D1/D2.** "13 of 14" describes the tree before the fix, not HEAD. | C-run |

A finding while reading for Q1: `Journaler::flush` holds the state lock **across** `store.append_events`
(`journal.rs:240-258`). That contradicts the type's documented discipline at `journal.rs:64-68` ("the state lock is
NEVER held across a redb store call"). The behaviour is the safe one, because flushes serialise. The comment is
wrong, and Q1's design relies on the behaviour, not on the comment.

## Q1. D3's barrier: time-scoped or scoped to process-blocking nodes?

**Time-scoped. But as a deadline on the oldest unflushed event, not as "at most once every N seconds".**

- **The rate-limit form is wrong, and wrong exactly where it matters.** "Flush at the `NodeStarted` append if
  ≥ N s since the last flush" skips every start that follows a recent flush. The acceptance write (spec 154) and
  every resume or suspend barrier is such a flush, so the first node of a run, or the first node after a resume,
  is never flushed. That is `check` after `spawn_resume`, the exact 064 case.
- **The effect-scoped form fits `write_amp.rs` by accident, not by construction.** Its lean graph has no
  `SPAWNS_PROC`/`NET` node, so it pays 0. A real graph of k fast process nodes pays +k commits regardless of
  duration. It also misses any node that blocks without declaring the effect.
- **The deadline form is bounded by construction.**
  - A per-run watcher thread (`std::thread::scope` around the scheduler loop, waiting on a `Condvar`) flushes when
    the **oldest pending event is ≥ N s old** while a node is in flight. N defaults to 2 s.
  - A run that finishes inside N pays **0** extra commits, whatever its node kinds and node count.
  - A node that blocks for T ≥ N pays **exactly 1** extra commit: the deadline flush empties `pending`, and
    nothing is appended again until the node returns. Progress does not go to the journal (Q3).
  - Total extra commits ≤ number of nodes that individually run ≥ N s. This is a count of slow nodes, not of
    nodes.
  - Concurrency is already safe: `flush()` takes the state lock for its whole duration (§0), so a watcher flush
    and a worker's batch flush serialise.

**Tests that pin it (all in `pii-graph`):**
1. **The existing `write_amp.rs`, unchanged:** `per_run < 220`. Add `assert_eq!(journaler.deadline_flushes(), 0)`
   over its 300 runs, so the zero is asserted, not merely within margin.
2. **"Not per node":** 300 runs of a graph with 20 instant nodes of a test kind declaring `SPAWNS_PROC`.
   `write_txn_count` per run must equal the same graph with the watcher disabled. An effect-scoped barrier fails
   this (+20 per run); the deadline form passes.
3. **"Visible while blocked":** one node sleeping 3 s (with N = 1 s). A reader thread polling `read_journal`
   sees `NodeStarted` within N + 200 ms of dispatch, and the run's commits are exactly the baseline + 1.
   - **This is the fail-first:** on HEAD, `NodeStarted` is invisible until the node returns, so the reader
     sees it at ≥ 3 s.
4. **Mutation:** set N to `u64::MAX`. Test 3 must go red and tests 1-2 stay green. That proves test 3 depends on
   the watcher, not on timing luck.

## Q2. Should heal_detect read the gate's recorded verdict? Who runs the gate, and when?

**Yes.** Two facts change the 064 answer from "should" to "must":

1. **There is no recorded verdict to read today.** `fresh-gate` writes its outcome only to
   `<export>/gate-output.log` (`fresh_gate.rs:271-272`), inside a per-run export directory.
2. **`silent-noop-check` is not a gate step.** `gate-steps.txt` has 0 occurrences of it. It runs only in
   `cargo xtask ci` (`xtask/src/main.rs:90`) and in the watch's own nested cargo. So the boss watch is today the
   **only** regular executor of the check it watches, and it executes it at the worst place.

**Change:**
- **R1. The gate owns the check.** Add `silent-noop-check` to `gate-steps.txt`. It is a source scan, and xtask is
  already built by earlier steps, so it costs seconds.
- **R2. The gate writes a durable verdict.** `fresh-gate` appends one line per run to `_tmp/gate/verdicts.jsonl`:
  `{rev, tree_dirty, ts, wall_s, pass, steps:[{label, pass, secs}], failing_tests}`. The file is append-only,
  owned by the gate, and outside the export.
- **R3. Who runs the gate, and when:** the builder loop, under the cycle lock.
  - It already runs `fresh-gate` per item.
  - Add one rule to `tick.sh`: if no verdict exists for `master` HEAD within 24 h and no cycle holds the lock,
    queue a gate-only cycle on HEAD.
  - The daemon never runs cargo.
- **R4. `heal_detect` reads, never spawns.** It routes on the newest verdict lines for its gate's step:
  - `ok`: the newest verdict passed;
  - `detected`: two consecutive verdicts on **different gate runs** failed with the same failing set. This
    replaces "run it twice in a row";
  - `flake`: a FAIL followed by a PASS on the same rev;
  - `stale`: no verdict within 24 h. This is the new honesty port, the sibling of `inconclusive`.
  - The graph drops `caps = ["proc"]`.

**Proof:**
- `grep -c silent-noop-check gate-steps.txt` returns 1 (HEAD: 0);
- after one gate run, `verdicts.jsonl` gains exactly one line, whose `rev` is `git rev-parse HEAD`;
- `heal_detect` unit tests over four fixture verdict files route to `ok`, `detected`, `flake` and `stale`;
- `boss-watch.toml` validates with `caps = []`;
- with boss-watch scheduled for a day, `ps --ppid <daemon>` never shows `cargo`.

## Q3. NodeProgress with a mandatory timeout for PROC nodes: the minimum shape

**Progress is state, not history. It does not go into the journal.** The journal receives only what resume needs:
`NodeStarted`, made durable by Q1's deadline flush, and the terminal event. Everything else is a live reading.

**Minimum shape:**
- **R1. One in-memory row per in-flight node** in the engine's run registry:
  `InFlight { run, node, kind, started_ms, timeout_ms, child: Option<{ pid, cpu_ms, out_bytes }> }`.
  - Q1's watcher updates it on the same N-second tick.
  - `cpu_ms` is read from `/proc/<pid>/stat` (utime + stime) of the child's process group. `ProcCap` already
    places the child in its own group (`childproc.rs:56`).
  - `out_bytes` is the length of the captured streams so far.
  - `ProcCap::run` needs one addition: a shared `ChildStats` handle in `Ctx`, set at spawn, which the watcher reads.
    Nothing is pushed per tick.
- **R2. Read through what exists.**
  - `run get` / `run.status` returns `in_flight: [InFlight]`, and `pii health` adds `nodes_in_flight`.
  - The observe file log gets at most one `node.in_flight` line per node per 30 s. That is a log write, not a
    redb transaction.
- **R3. A bound, declared:** a node kind whose `effects()` include `SPAWNS_PROC` must carry `timeout_ms` in its
  config, or validation refuses the graph.
  - `heal_detect`'s hard-coded `Duration::from_mins(2)` (`heal.rs`) becomes that field. This becomes moot if Q2
    lands, since the node then spawns nothing.
- **R4. The rule for readers:**
  - `cpu_ms` or `out_bytes` rising means working;
  - both flat for ≥ `min(timeout_ms / 4, 60 s)` means `wedged?`;
  - elapsed > `timeout_ms` means the node has already failed with its timeout.

**Proof:**
- With a node running `sh -c 'while :; do :; done'` for 5 s, two `run get` reads 1 s apart show `cpu_ms` rising.
  With `sleep 5`, `cpu_ms` is flat and the reader prints `wedged?`.
- During a 30 s node, `store.write_txn_count()` rises by **exactly 1**, from the deadline flush. This pins "no
  transaction per progress event".
- The file log holds ≤ 2 `node.in_flight` lines for that node.
- A graph with a `SPAWNS_PROC` node and no `timeout_ms` is refused at `Engine::load_spec`. HEAD accepts it, so the
  check is red on HEAD.

## Q4. Is pii production-ready for a release build and install?

**No.** The deployed daemon is broken right now, and nothing has told anyone. Measured today:

| fact | value | label |
|---|---|---|
| Deployed daemon | pid 51837, up 20.9 h since 10-16 01:18:23. Revision **`a5e51fb13cd6-dirty`**, the ungated leftover commit from 063, still. | C-run (`pii health`) |
| Its Discord channel | **dead since 10-16 04:47:47 UTC, 3 h 29 min after start.** 25,301 + 18,614 `channel.poll.failed` events, "capability denied: net: **byte budget exhausted**", 2 per second, through 22:15. `pii health`: `errors: 43918`, `cursor: 0`, `runs: 0`. **The 09:00 UTC digest did not run** (0 runs). | C-run (both log files) |
| Cause | `ConfigGrants::grant_with` sets `max_bytes_in/out = 16 MiB` (`daemon/config.rs:698-699`). It is **cumulative for the life of the process** (`pii-net/src/service/conn.rs:160-188`, the counters never reset). A 500 ms poll spends about 1.3 KB/s, i.e. 16 MiB in about 3.5 h. **The channel dies about 3.5 h after every start, deterministically, until the next restart.** | C-read, arithmetic over C-run timestamps |
| Detection of it | HEAD's reader (156 + 160, `target/debug/pii`, built 10-16 07:31), run on copies of both log files: `starts_considered=6 incidents=0`. The rules cover restarts, unready starts, stalled runs and run/worker refusals. **`channel.poll.failed` and `channel.egress.denied` are not incident sources.** | C-run |
| Supervision | The daemon's ppid is now 0 (outside the container's pid namespace); on 10-15 it was a Hermes session. In neither case does anything restart it. | C-run |
| The gate on HEAD | No full gate on disk since D1/D2 (§0). | C-run |
| Disk | `/projects` is 96% used, 24 G free. `target/` is 57 G, `_tmp/gate-target` 22 G, `/projects/_tmp` 142 G. A coverage-instrumented gate needs about 18 G (asserted, `gate-steps.txt`). A release build with fat LTO on top of that may not fit. | C-run |
| `pii-mcp` | swapped: the installed `pii-mcp` is 2,229,224 bytes, dated 10-15 19:05, the same as yesterday's `.new`. Its effect on Hermes's error share is not measured. | C-run |

**What must be true first.** Each item is a measurement, and each is red today.
1. **One `fresh-gate` 14/14 on a clean HEAD, run by the loop and recorded** (Q2 R2): one `verdicts.jsonl` line
   with `pass: true`, `tree_dirty: false`, `rev == HEAD`.
2. **The channel survives a day.**
   - The net byte budget becomes a **rate** (bytes per window, refilled) or is reset per poll. Proof: a
     scripted-transport test runs 50,000 polls with no `byte budget exhausted`, and a single response larger than
     the window is still refused.
   - Separately, the forum ingress, which can read nothing (063 Q3(4)), is unbound until it points at a thread.
     That removes 2 requests/s of pure spend.
3. **The reader sees this incident.** `channel.poll.failed` with the same `error` N times within the window opens
   one incident. Proof: on a copy of today's `pii.log.1`, at least one incident, `first_seen` 10-16 04:47:47,
   opened within the window. HEAD prints `incidents=0`.
4. **The release is built from a clean tree whose revision has a PASS verdict:** `pii health` `revision` has no
   `-dirty`, and equals the `rev` of a `pass: true` line.
5. **Supervised:** `kill -9` brings it back within 60 s under a process that is not an agent session.
6. **Headroom:** at least 40 G free before the release build. That needs a decision on `/projects/_tmp` (142 G),
   which is the operator's call under the no-delete rule.
7. **The way back is real:** `deploy/rollback-pii.sh` is rehearsed once on the host against the actual previous
   binary, and the deploy itself goes through the 152 card (Q5).

**What would make it false after install (the 24 h soak):**
- `channel.poll.failed` rate > 0/h after the first hour;
- `channel_posts` lower after a restart than before it;
- the 09:00 UTC digest absent;
- any start without a matching `daemon.started`;
- Hermes's `mcp__pii__*` error share over 7 days above the 7 days before the swap.

## Q5. The four items closed by hand: re-gate them, or accept them on their own criteria?

**Re-gate first, as one batch, then annotate. Do not choose between the two.**

Why one batch suffices: the gate judges a **tree**, not a spec. Every one of 152, 156, 159 and 160 is in HEAD, so
a single `fresh-gate` PASS on a clean HEAD gates all four at once. That is one warm gate run, not four dispatches.

State check (C-run, `state.json`):
- A21 (152), A23 (156) and A26 (159) read `done`, "verified by hand".
- A27 (160) and A22 (155) read **`parked`**, so 160 is not "closed" in the loop's own record.
- `c0b3601` names the boss-watch test as the blocker of both parked rows, and that test is green on HEAD (asserted).

**The honest order:**
1. **Clean tree and headroom** (Q4 items 6 and 1). The gate cannot be trusted on a 96%-full volume.
2. **Close D2's evidence gap by mutation, not by history.** The pre-fix run failed to compile, so that run proves
   nothing. Keep the `timed_out` field, delete only the early `inconclusive` return in `HealDetectNode::run`, and
   run the proof test.
   - It must fail: the attempt falls through to the retry and reaches `detected` or writes a heal row.
   - Restore the line, and it passes.
   - That is the behavioural fail-first, at the cost of one test run in a scratch copy.
3. **One `fresh-gate` through the loop on HEAD**, recorded (Q2 R2). This requeues A22 and A27, which `c0b3601`
   already planned.
4. **Then annotate each closure with the verdict line:** "closed on its own criteria by hand; tree-gated at `<rev>`
   by `<verdicts.jsonl line>`". If the gate fails, the failing step says which closure, if any, is implicated, and
   only that one reopens.

**What re-gating cannot certify, said plainly:**
- **160** meets its own criteria. On today's real log it sees all 6 starts and opens 0 false incidents (C-run,
  §Q4). But it reports `incidents=0` while production's channel has been dead for 17.5 h. The closure is right on
  its criteria and insufficient for production. That is a new spec (Q4 item 3), not a reopen.
- **152** passes its tests, but the one real deploy did not go through the card (063 §0). Its claim, "the operator's
  one permission", is proven only when the next deploy (Q4 item 7) goes through it. Accept the code and leave the
  claim open.
- **156** is partly superseded by 160, since 160's rotation and audit-log rules replace 156's false positives.
  Gating it is still meaningful as part of the tree.
- **159** measures operator distance. Gating proves it builds and its tests pass, not that the number is right.
  Check that one by hand once: `pii incidents distance --since 2026-10-11` against the git count in 063 Q2.

## Final ranking (by effect)

| rank | change | the measurement that proves it | red today |
|---|---|---|---|
| 1 | **Fix the net byte budget** (a rate, or reset per poll), and unbind the forum ingress | 50,000 scripted polls with 0 `byte budget exhausted`; on the host, `errors` flat over 24 h | 43,918 errors; dead 3 h 29 min after start |
| 2 | **The reader opens an incident on repeated `channel.poll.failed`** | on a copy of today's `pii.log.1`, ≥ 1 incident with `first_seen` 04:47:47 | `incidents=0` |
| 3 | **Durable gate verdict, `silent-noop-check` as a gate step, one loop-run gate on clean HEAD** (Q2 R1-R3, Q5) | one `verdicts.jsonl` line, `pass: true`, `rev == HEAD`; `grep -c silent-noop-check gate-steps.txt` = 1 | no verdict file; last full gate 10-16 11:16 FAILED; 0 occurrences |
| 4 | **D3 as a deadline flush with a per-run watcher** (Q1), and progress kept in memory (Q3) | `write_amp` unchanged with `deadline_flushes == 0`; the 20-node graph's commits equal the baseline; a 3 s node's `NodeStarted` visible within N + 200 ms; +1 commit per 30 s node | the +1-per-node barrier, reverted at 304/100 |
| 5 | **`heal_detect` reads verdicts and spawns nothing** (Q2 R4) | `caps = []` validates; four fixture verdicts route to their four ports; 0 `cargo` children of the daemon per day | still shells `cargo xtask` |

Release and install come **after** 1-3. They come through the 152 card, from a clean tree whose revision has a PASS
verdict, with at least 40 G free and a supervisor in place (Q4 items 4-7).

## NOT VERIFIED

| item | why |
|---|---|
| D1's 0.41 s under 32 hogs and 8/8 green | taken from the brief and `c0b3601`; not re-run, to avoid loading a 96%-full shared host |
| Why the 09:00 digest left no run at all, rather than a refused post | there is no cron event in either log file; I did not trace the scheduler |
| Bytes per poll | inferred from 16 MiB over 12,569 s at about 2 polls/s; not measured per request |
| Whether the daemon's ppid 0 means `podman exec` or a reparent | it is outside the container's pid namespace |
| Hermes's error share since the `pii-mcp` swap | not queried |
| The D2 mutation result | proposed, not run: it needs an edit to `heal.rs`, which is outside this read-only brief |

**Scratch:** `/projects/_tmp/rev065-QS7b/` (35 M, log copies) is from this run. **`/projects/_tmp/rev064-4TDe/`
(4.5 G)**, from the previous round, holds the export and two build targets. It is mine and can go. I have not
deleted it; given 24 G free, that is the operator's call. Nothing in the repo changed except this file, and nothing
was committed.
