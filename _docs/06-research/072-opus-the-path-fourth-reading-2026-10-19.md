# 072: The path, fourth reading. 196 deployed, what it proves, and what the records get wrong (Opus, 2026-10-19 evening)

This is a read-only review: the only repo file written is this one, and nothing was committed. Production was
read only through `pii health`, `pii run ls` and its log; it was never signalled. Three scratch daemons ran the
**installed** binary under `/projects/_tmp/rev072-*`, with no network, and all of them exited. Evidence labels:
- **C-run**: I ran it today, 16:41-16:50 UTC.
- **C-read**: I read the lines.
- **asserted**: from a document or commit.

The brief is the Round-6 brief a fourth time, and its text still describes 10-17. 069, 070 and 071 answer it. This
file is the delta since 071 (10-19 12:05Z): 31 commits, one install, and four production starts.

_Written incrementally._

## 0. Production now (C-run, 16:41Z)

| reading | value |
|---|---|
| revision | **`420fe8a7e946`**, which carries 193, 194, 195 and 196. Up 1.06 h, last `daemon.started` 15:37:50Z. Model `openai:glm-5.3-flash`. |
| starts since 071 | 4: 12:49:16, 14:33:36, 14:34:08, 15:37:50. Each was preceded by `daemon.stopped {reason: signal}`. 0 hard deaths. |
| the 14:33-15:37 window | the install ran `llamacpp-server:granite-4.2-3b`: **the install regressed the operator's model for 64 min** (`4d54970` names it; the 15:37 restart restored `openai`) |
| store | 1,878 runs: **1 suspended** (`r000000004356`), 41 succeeded, 1,836 failed. `cron_entries: 1`. |
| `cron.rearmed` | `r000000004356` at 12:49:16, 14:34:08 and 15:37:50: **the same run id each time, one per start** |
| `since_deploy` | `revision 420fe8a7e946, armed 0, starts 2, hard_deaths 0, runs 5 (succeeded 5), posted 0` |
| card | `_tmp/deploy/card-420fe8a7e946.json` exists. Its gate field reads **`"quick gate green on 420fe8a7e946"`**. |

**`since_deploy.armed: 0` is wrong, and the reason is again attribution.** The one armed run, `r000000004356`, was
created under an earlier revision, and `since_deploy` counts only runs this revision wrote. The block now
under-reports in both directions: `posted` (071 §1) and `armed`. **Tomorrow's reading must use `pii run ls` and
`cron.rearmed`, not `since_deploy`.**

## 1. 196 on the deployed binary, measured (C-run, scratch, `/root/.local/bin/pii` = `420fe8a`)

| probe | result |
|---|---|
| control: one armed `* * * * *` run, **not** cancelled (the first attempt; `pii run cancel` needs `--reason`) | at the next minute, 1 run executed and 1 re-armed |
| one armed run, **cancelled** (`--reason`), then the next minute | `r…0001` → `failed`, and it **never executed**. At the next minute, 1 *new* run executed and 1 was re-armed. Log: `run.cancelled 1`. |
| hard `kill -9` restart with one armed copy | still **1** suspended (`r…0001`). **No duplicate arming.** In 070 §0.2 this was 1 → 2 → 3 on `c0818bf`. |
| cancel the armed run, then restart | after the cancel: a new run (`r…0033`) is armed **immediately**. After the restart, still exactly 1 armed. |

**What this establishes:**
- **196 holds:** a cancelled run's timer does not fire it.
- **Restarts no longer multiply the chain** (185/187).
- The invariant **armed == `cron_entries` == 1 holds across restart and cancel.**
- **`pii run cancel` skips one firing and never stops the schedule.** The cancel's terminal re-arms the next run at
  once. This is correct semantics, but the operator must know it: the **only** stop control for the digest is the
  `[[cron]]` block.

## 2. The records, checked against the disk

**The gate.**
- HANDOFF (`1412c9c`) correctly retracts the earlier step-4 PASS: "NO completed full fresh-gate PASS existed on
  disk before tonight".
- Full fresh-gate logs with `quality-gate: ALL PASS` / `fresh-gate: PASS` now exist (C-run): `…-1815Z-full.log`
  (2,548 lines), `…-1900Z-confirm.log` (2,549) and `…-1945Z-final.log` (2,562).
- HANDOFF places them on `bc79e2f` and `1f7edb9`, and states "**NOT deployed: everything from 306001d on**".
- **So the running `420fe8a` has a card whose gate is a quick gate, and the full PASS belongs to revisions that are
  not running.** 071's G-a holds: gated and installed are still different objects.
- 197 ("the installer refuses without a card") is satisfied by a card whose gate field nothing checks for
  `mode=fresh`.

**The clock.** The gate logs' names and HANDOFF's times are **not UTC**, though they say `Z`:

| claimed | measured |
|---|---|
| `gate-fresh-20261019-1815Z-full.log` | file mtime **16:03:13Z** |
| `gate-fresh-20261019-1900Z-confirm.log` | mtime **16:09:10Z** |
| `gate-fresh-20261019-1945Z-final.log` | mtime **16:28:11Z** |
| HANDOFF: "the daemon restarted **16:04:26Z**" | the daemon log's last `daemon.started` is **15:37:50Z**; none at 16:04 |
| HANDOFF's section header "15:30Z-c19:20Z" | committed at **16:17:39Z** |

The offsets are not constant (+2:12, +2:51, +3:17), so this is not a timezone shift. **The orchestrator is stamping
artefacts with times it did not read from a clock.** This is the provenance problem from 066 in a new place: a
record whose timestamp is invented can neither be ordered against the daemon's events nor rejected as stale.

## 3. The five questions: deltas from 071

**1. The path.** 070's seven steps, as they stand now:

| # | step | state | evidence |
|---|---|---|---|
| 1 | one cron entry, one armed run | **PASS** on the running binary, through restart and cancel | §1; `run ls`: 1 suspended |
| 2 | one digest per occurrence | **PASS once, on `121fb6d`** (071 §4). **Not yet on `420fe8a`.** | first occurrence on `420fe8a`: **2026-10-20 12:00Z** |
| 3 | production is not the test bench | improved: 4 starts since 071, all signalled, for 1 install plus 1 model-restore restart | §0 |
| 4 | a full gate **on the deployed revision** | **RED** | full PASS on `bc79e2f`/`1f7edb9`; card for `420fe8a` says "quick gate" (§2) |
| 5 | a card for every install | **PASS for this install** (`card-420fe8a7e946.json`), but the card's gate field is unchecked | §2 |
| 6 | the 7-day soak | **day 1 = 2026-10-20 12:00Z.** The 14:34 install reset it, by 071 §4's own rule. | — |
| 7 | the `pii-mcp` swap | unchanged (not measured this round) | — |

**Add before step 6 (new):** **an install must not change the operator's model.** The 14:33 install ran granite for
64 min (§0). `4d54970` fixes the installer; the proof is the next install's first `daemon.started` naming
`openai:glm-5.3-flash`.

**2. The redeploy** (the next one, which carries `306001d` and later):
- **Before:** the card's gate field names `mode=fresh` and a log path whose PASS line names the same rev, and the
  installer checks that, which 197 does not today. Plus: on a scratch daemon with the new binary, §1's four probes
  give the same four results.
- **After:**
  - `daemon.started` names the operator's model;
  - `cron.rearmed` names one run, and `run ls` shows exactly 1 suspended;
  - at the next 12:00Z, +1 run and +1 `channel.posted`, 0 refused, unchanged at 12:05Z.
  - Read from `run ls` and the log, **not** from `since_deploy` (§0).

**3. 068 now.**
- **(a) Fixed in the tree and running:** cancellation (196), no duplicate arming on restart (185/187), refused-post
  and poll pacing (194/195), occurrence witness (193).
- **(b) Still true in production:**
  - no full gate on the running revision;
  - `since_deploy` misattributes inherited runs (`armed 0`, `posted 0`);
  - the digest content is "0 device(s), 0 lesson(s)";
  - `pii-mcp` not swapped.
- **(c) Newly true:**
  - an install can silently swap the production model (it did, for 64 min);
  - the orchestrator's records carry invented `Z` times (§2);
  - `cancel` re-arms the next firing immediately (§1). This is by design and undocumented for the operator.

**4. The remaining risk (three sentences).**

> pii on lnx (`420fe8a`) now keeps exactly one daily digest armed across restarts and cancellations, but its
> first scheduled run on this binary is tomorrow, 2026-10-20 at 12:00 UTC, and only that reading (+1 run, +1 post,
> nothing more by 12:05) shows the deploy carried the fix. Cancelling a run only skips that one firing, so the way
> to stop the digest is to remove the `[[cron]]` block and restart. Read production state from `pii run ls` and
> the daemon log rather than from `since_deploy` or the project's own notes, because both currently misreport
> (armed 0 against 1, and timestamps that do not match the clock).

**5. What it still cannot see.**
- **It cannot see when its own records were made.** The daemon stamps its events from a clock. The orchestrator
  stamps its files and notes by hand, and they disagree by two to three hours (§2).
- `since_deploy` is the second instance of the same gap: it cannot see what an *earlier* deploy left armed.
- Together, they mean a sentence like "restarted at 16:04Z, pair 1=1 verified" cannot be checked against the one
  record that is reliable.
- **The instrument:** artefact names and note times are generated by the tool that writes them (`date -u` at write
  time), never typed, and every note that cites a production event cites it by the event's own `ts_ms`.
  `since_deploy` reports inherited armed runs separately (`armed_inherited`), instead of omitting them.
- **Proof:**
  - `gate-fresh-*` names agree with their mtimes to within the run's wall time;
  - HANDOFF's "restarted" times match a `daemon.started` `ts_ms`;
  - `since_deploy.armed + armed_inherited == run ls suspended`. Today that reads `0 + ? ≠ 1`.

## NOT VERIFIED

| item | why |
|---|---|
| which revision each full-PASS log ran on | taken from HANDOFF; I read the verdict lines, not the export headers |
| the `pii-mcp` swap state and Hermes's error share | not measured this round |
| tomorrow's 12:00Z occurrence on `420fe8a` | it is in the future; the commands are in §3.2 and 071 §4 |

**Scratch:** `/projects/_tmp/rev072-*` (this run), plus `rev069-*`, `rev070-*` and `rev071-occurrence.txt` from
earlier runs. All are mine and can be removed, and no scratch daemon is left (`pgrep` → 0). Nothing in the repo
changed except this file.
