# 070: The path, re-measured after the redeploy (Opus, 2026-10-18)

This is a read-only review: the only repo file written is this one, and nothing was committed. Two scratch
daemons ran under `mktemp -d` (`<scratch>/rev070-*`), with no network; both exited, and the production pid
2139213 was never signalled. On production I used only `pii health`, `pii run ls` and `pii run get`, which are
read-only RPCs. Evidence labels:
- **C-run**: I ran it today, 11:54-12:30 UTC.
- **C-read**: I read the lines.
- **asserted**: from a document or commit message.

The brief repeats Round 6, and 069 (10-17) answers it for the tree as it stood then. Since then (19:45 UTC), **30 commits** (4 of them leftovers)
have landed and **six installs** have happened. This document answers the same five questions **for what is
running now**, and corrects 069 and 068 where the night proved them wrong.

_Written incrementally._

## 0. What happened between 069 and now (all C-run unless marked)

| fact | evidence |
|---|---|
| 175 committed whole: `resolve_graph_dir` is now in HEAD's `lib.rs`, and `tests/daemon_graphs.rs` is tracked | `6f76a45`; `git grep -c 'fn resolve_graph_dir' HEAD` → 1; `git ls-files …daemon_graphs.rs` → listed |
| the digest graph has a timer: installed and repo copies have 1 `timer` node, `cron = "0 9 * * *"` | `grep -c 'kind = "timer"'` → 1 |
| **the daemon refuses a timer-less cron graph**: on the deployed binary, a scratch config with a `transform`-entry cron exits `invalid: cron 'probe': graph 'loop' must start at a timer node (entry is transform); it would re-run as fast as it finishes` | scratch `rev070-*/notimer` |
| **a timer cron graph holds**: the same binary, a timer-entry graph, gives `runs` 0 → 1 → 1 at 0, 6 and 10 s, and `run ls` shows it `suspended` | scratch `rev070-*/timer2` |
| **six installs since 10-17 21:54**, by aside directory: 21:54, 22:04, 22:32, 23:34, 02:08, 11:10 | `ls -d ~/.local/bin.old-*` |
| revisions announced by the daemon's own start post: `b61c65909289-dirty`, `7d2410172f44`, `2ebcf0f8f7f9`, `60553615f85f-dirty`, **`?` (02:08 to 11:10)**, `c0818bfcc898` | `channel.verification_posted` text in `pii.log` and `pii.log.1` |
| **85 daemon starts since 10-17 21:54, 72 of them with no `daemon.stopped` before the next start**, i.e. hard deaths, 76 of them between 21:00 and 23:00 | `pii.log*` (§0.1) |
| **the 09:00 UTC digest today**: 10 `channel.posted` (09:00:02-09:00:03), then **1,836 `channel.post_refused` with Discord `429`** (09:00:03-09:02:49) | `pii.log` |
| **the store holds 1,850 runs, all of one graph (the digest): 10 succeeded, 1,836 failed, 4 suspended** | `pii run ls` (read-only) |
| production now: `c0818bfcc898`, no suffix, up 0.6 h, `runs` flat at 1,850 over 10 s, one `daemon.graphs_fallback` at its start | `pii health --json`, `pii.log` |
| **one deploy card on disk for six installs**: `_tmp/deploy/card-b61c659.json` | `find _tmp -name '*card*'` |
| **no full-gate PASS recorded since 10-17 02:01.** The last measurement, `measure-gate-tests-20261018T061826Z.log`, reads `MEASURE full-workspace rc=101`; `bb43007` (06:28) says "the suite is red at HEAD and no gate runs it" | `_tmp/autonomy/logs/` |
| `pii-mcp` still not swapped: `pii-mcp` is 10-15 19:05, and `pii-mcp.new` is 10-18 11:10 | `ls -la ~/.local/bin/pii-mcp*` |

### 0.1 What the 09:00 event was

- **It was not a retry storm.** It was about 1,846 digest runs at one occurrence: 1,836 failed and 10 succeeded.
  `run ls` shows one graph hash for all of them.
- The successes took 2.0-3.4 s each. The failures ran a median of 86.6 s and up to 169.5 s, which is consistent
  with each run waiting on Discord's `retry_after=132s` while new runs kept starting.
- **The 10 successes have ids spaced by 32** (`r…0001`, `r…0033`, … `r…0321`). The store allocates ids in blocks of
  32 per process, so they came from 10 different daemon processes. **Each start had armed its own copy of the one
  daily cron, and all of them fired at 09:00.**
- **The same pattern is armed again now:** 4 suspended digest runs (`r…4129`, `r…4161`, `r…4193`, `r…4225`), from 4
  consecutive id blocks, for `cron_entries: 1`.

**Corrections to my own earlier documents:**
- **068 §1 correction 2 is partly withdrawn.** 72 starts with no prior `daemon.stopped` were each followed by a new
  start. Something does restart the daemon after a hard death. The supervisor itself is still not visible from this
  container.
- **069 §0.2's loop happened in production**, at a lower rate than the 58 runs/s I measured in scratch. Each run
  blocked on the 429 instead of finishing in milliseconds.
- **`EXERCISE-MATRIX.md` (ee0a1a5, 11:52) is wrong in three places:**
  - **E10 is marked PASS** ("+0 runs in 60 s with the cron live") **three hours after its own occurrence produced
    ~1,846 runs.**
  - Findings 1-2 ("`channel.poll.failed` 6,672", "`channel.egress.denied` 6,656 — the byte budget still denies under
    the per-connection fix") **count log history from 10-17 01:02-03:46**, written by the pre-fix
    `a5e51fb-dirty` binary (068 tally). Since 10-17 20:00 the log has **0** `channel.egress.denied`.
  - Finding 3 attributes the 1,836 × 429 to outbound pacing (E6). It is the scheduler multiplying runs.

### 0.2 The scheduler, tested on the deployed binary (C-run, scratch, no network)

Three scratch daemons, each running `~/.local/bin/pii` (`c0818bf`), with a `* * * * *` timer cron:

| probe | result |
|---|---|
| **one armed run, a node that succeeds** | exactly **one run per occurrence**: runs 2 → 3 → 4 at 12:01, 12:02 and 12:03, with 1 suspended between occurrences |
| **one armed run, a node that fails** (post to a channel that does not exist) | **one failed run per occurrence**, then re-armed. No storm from failure alone. |
| **three `kill -9` restarts on the same store, then one occurrence** | suspended runs **1 → 2 → 3** (each start arms another copy). At 12:06 **all 3 executed** (succeeded 1 → 4), then the chains collapsed to 1 suspended. |

**What this establishes:**
- **One occurrence executes once per armed copy, and every start arms another copy.** Production holds **4** armed
  copies now (§0.1). **Predicted for 2026-10-19 09:00 UTC: 4 digest runs and 4 forum posts, not 1.**
- After that occurrence the chains collapse to one, so 10-20 should post once, unless the daemon restarts in
  between.
- **No production scheduler code changed after the build that stormed.** `git log 136b23e..c0818bf -- crates/*/src`
  is empty, and **184 changed only a test** (`tests/cron_runtime.rs`, +28/−23).
- What these probes do **not** reproduce is the 09:00 multiplication from 10 armed copies to ~1,846 runs. Each
  real run blocked up to 170 s on Discord's `retry_after`, and some path started new runs while those were in
  flight. The lag-rescan path (spec 79/88) is the candidate, and it is unproven. **That cause is still open, and
  it is the one that turns 4 posts into a flood.**

## 1. The path, from what is running now

The order is set by what earns the verdict. The scheduled path is first because it is the only production
behaviour that has failed twice, in opposite directions: 0 runs in 14 h (068), then ~1,846 runs at one occurrence
(§0.1).

| # | step | proof that closes it | today |
|---|---|---|---|
| 1 | **One cron entry, one armed run.** A daemon start must not arm a second copy while a suspended run for that entry exists, whether by deduplicating on the entry name in `start_cron_entries` or by cancelling the stale ones at boot. | `pii run ls \| jq '[.[]\|select(.status=="suspended")]\|length'` equals `cron_entries`, after 3 consecutive restarts in a **scratch** daemon | **4 vs 1** (red) |
| 2 | **The occurrence itself: exactly one digest.** This is 184's claim ("re-arm waits for the next occurrence"), and it is unproven at a real occurrence. | at 09:01 UTC: new digest runs **== 1**, new `channel.posted` **== 1**, `channel.post_refused` **== 0**; at 09:05, still 1 | first chance: 2026-10-19 09:00 |
| 3 | **Production stops being the test bench.** Matrix exercises (E1, E3, …) run on scratch daemons. Today's `cron.start.refused 'no-such-graph'` (11:18:50) and the 5 starts between 11:10 and 11:19 happened on the live one. | production `daemon.started` count over 24 h == installs over 24 h | 85 starts against 6 installs |
| 4 | **A recorded full gate on the deployed revision**, from a clean export, quoted by its card | a `fresh-gate: PASS` line naming `c0818bf` (or its successor), and the full workspace suite rc=0 | last full PASS 10-17 02:01; full suite rc=101 at 06:18 |
| 5 | **A card for every install.** `install-pii.sh` refuses to run without `_tmp/deploy/card-<rev>.json` for the rev it builds. | `ls ~/.local/bin.old-* \| wc -l` == number of card files, from now on | 1 card for 6 installs |
| 6 | **The soak: 7 days counted from the store, not from per-process counters** (§5) | over 7 × 24 h: exactly 7 digest posts, 0 refused posts, 0 starts without a prior `daemon.stopped` except for installs | not started |
| 7 | **The `pii-mcp` swap**, behind its probe | Hermes `mcp__pii__*` error share over 7 days after ≤ 7 days before | `pii-mcp.new` staged, not swapped |

**What earns the verdict is steps 2 and 6.** The other steps make those two readings trustworthy.

## 2. The redeploy: 069's checklist against what actually happened (`c0818bf`, 11:10)

| 069 check | result | evidence |
|---|---|---|
| B1 committed tree builds and holds the fix | **PASS** | `6f76a45`; the tree is clean (`git status --short` empty at 11:54) |
| B2 digest scheduled by a timer | **PASS** | installed graph: 1 timer, `0 9 * * *` |
| B3 full fresh-export gate PASS on that revision | **FAIL** | no PASS recorded since 10-17 02:01; full suite rc=101 at 06:18 |
| B4 the loop reproduction does not loop | **PASS** (C-run today) | timer-less cron refused; timer cron 1 suspended, flat over 10 s |
| B5 rollback target present | **PASS** | `~/.local/bin.old-1792321624` (11:10) |
| card for the deploy | **FAIL** | no `card-c0818bf*.json` |
| A1 revision, no suffix | **PASS** | `c0818bfcc898` |
| A2 `daemon.graphs_fallback` at start | **PASS** | 1 event at 11:19:02 |
| A3 runs ≤ 1 in the first minute | **PASS for growth, FAIL for arming** | `runs` flat at 1,850, but 4 suspended digests are armed for 1 cron |
| A4 exactly one digest at 09:01 | **pending** | 2026-10-19 09:00 UTC |
| A6 starts paired with stops | **FAIL** | 72 of 85 starts had no stop |

**The commands, and the numbers that would make me say the deploy carried the fix:**

```sh
# before 09:00 UTC tomorrow (expected now: 4; carried-the-fix: 1)
PII_RUNTIME_DIR=/run/user/0/pii pii run ls | jq '[.[]|select(.status=="suspended")]|length'
# at 09:01 and 09:05 UTC (carried-the-fix: +1 run, +1 posted, +0 refused, unchanged between the two reads)
PII_RUNTIME_DIR=/run/user/0/pii pii run ls | jq 'group_by(.status)|map({(.[0].status):length})|add'
jq -c 'select(.kind=="channel.posted" or .kind=="channel.post_refused")|.kind' ~/.local/state/pii/pii.log | sort | uniq -c
# if more than 1 new run, or any post_refused, by 09:05: STOP THE CRON, do not roll back.
#   The rollback target (~/.local/bin.old-1792321624) is the `?` build that stormed at 09:00 today.
#   The safe act is to comment out the [[cron]] block in ~/.config/pii/pii.toml and restart the unit.
```

**My prediction, to be falsified:** tomorrow's occurrence executes **4** digest runs, one per armed copy, as
reproduced in §0.2. The 09:00 multiplication path is not fixed, so it may produce many more. To prevent it before
09:00, someone must cancel 3 of the 4 suspended runs, or deduplicate arming in code (§1 step 1), and keep the
daemon from restarting until then.

## 3. What 068's verdict says now

**Still: deployed, not production-ready.** The reason has moved from "it never runs" to "its one scheduled job
has never run correctly at an occurrence".

**(a) Fixed in the tree** (each in HEAD and in the deployed `c0818bf`):
- graphs are found: `6f76a45`, and the `daemon.graphs_fallback` event at the 11:19 start (C-run);
- a timer-less cron is refused at start (C-run, scratch, deployed binary);
- refusals are loud: `177`, and production logged `cron.start.refused 'no-such-graph'` at 11:18:50 (C-run);
- the revision survives a clean export: `c0818bf`, no `-dirty` (C-run);
- the write window is per connection (`181`), and 0 `channel.egress.denied` since 10-17 20:00 (C-run);
- re-arm "waits for the next occurrence" (`184`). **In the tree, not proven at an occurrence.**

**(b) Still true in production:**
- `pii health` reports `channel_posts: 1` while the store holds 10 posted and 1,836 refused digests from today. The
  counter is per process, so **health cannot see today's incident**;
- no card and no recorded full-gate PASS for the running revision;
- `pii-mcp` not swapped;
- the ingress polls the forum (`cursor: 0`);
- the digest content is still "0 device(s), 0 lesson(s)", because nothing writes entities (059 F9);
- unplanned starts: 85 starts against 6 installs.

**(c) Newly true because of the fixes:**
- **the scheduled path runs, and at its first occurrence it multiplied: ~1,846 runs, 10 forum posts, 1,836
  refusals** (§0.1);
- **the same multiplication is armed again: 4 suspended digests for 1 cron**;
- making graphs loadable exposed the cron-run panic (`340649d`: no Tokio reactor at `runs.rs:71`), and the
  supervisor restarted it 76 times between 21:00 and 23:00;
- production became the exercise target (§1 step 3).

## 4. The remaining risk, in writing (three sentences)

> pii on the host now runs its graphs (revision `c0818bf`, no suffix), but its first scheduled digest this morning
> posted 10 times and was refused 1,836 times by Discord, and the running binary has the same scheduler code
> with 4 armed copies of that one daily job, so tomorrow's 09:00 UTC run is expected to post at least 4 times
> (measured in scratch: each start arms one more copy, and all copies fire). Before 09:00, disable the `[[cron]]`
> block or cancel the extra suspended runs; do not roll back, because the rollback target is the binary that caused
> this morning's flood. Count nothing else as production evidence until seven days of exactly one
> digest per day, with no unplanned restarts, have been read from the daemon's own store.

## 5. What this project still cannot see about itself

**It cannot attribute an event to the deploy that caused it.**
- A run in the store carries `dur_ms, graph, run_id, seq, status`: no revision and no timestamp (`pii run ls`
  keys, C-run).
- `pii health` counters reset per process: `channel_posts: 1` today, against 10 posts in the store.
- The logs rotate across revisions. So three readers today reported the wrong thing with confidence:
  - **the exercise matrix** marked E10 PASS after E10 failed, and counted pre-fix byte-budget denials as current;
  - **`pii health`** showed no trace of the 09:00 event;
  - **the run ids** are the only clue that 10 posts came from 10 processes (§0.1), and that clue came from block
    allocation arithmetic, not from any field.

**The instrument that closes it, precisely:**
- every run record carries `{revision, process_start_ms, created_ms}`;
- `pii health` adds a `since_deploy` block read **from the store**, keyed by the running revision:
  `{runs_by_status, posted, post_refused, starts, hard_deaths}`;
- each `[[cron]]` gets an occurrence assertion, `runs_per_occurrence == 1`, checked by the runtime reader.

**Proof:** replayed on today's store, the `?` revision reads `runs 1,846 / posted 10 / refused 1,836`, and
`runs_per_occurrence` reads 1,846 against an expected 1. `c0818bf` reads `runs 0 / armed 4`, against an expected 1.

## NOT VERIFIED

| item | why |
|---|---|
| that 184 deduplicates the 4 armed runs at fire time | the first occurrence under `c0818bf` is 2026-10-19 09:00 UTC; I did not read 184's diff to predict it |
| the forum's actual state (10 threads?) | the Discord API was not called, deliberately |
| the `pii/` keys in the workspace memory | not reachable from this shell, as in 069 |
| the supervisor's identity | still outside this container's pid namespace |

**Scratch:** `<scratch>/rev070-*` (this run) and `<scratch>/rev069-8e5o` (the previous run) are mine
and can be removed. No scratch daemon is left (`pgrep` → 0). Nothing in the repo changed except this file.
