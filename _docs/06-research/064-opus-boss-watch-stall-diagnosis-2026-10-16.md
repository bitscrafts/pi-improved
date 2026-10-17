# 064: The boss watch stall under load, diagnosed (Opus, 2026-10-16)

This is a read-only review. The only repo file written is this one, and nothing was committed. Scratch work is in
`/projects/_tmp/rev064-4TDe/`. Evidence labels:
- **C-run**: I ran it today.
- **C-read**: I read the lines.
- **asserted**: from the brief or the defect record.

Inputs:
- `DEFECT-2026-10-16-boss-watch-stalls-under-load.md`
- `graphs/starter/boss-watch.toml`
- `nodes/heal.rs`
- `tests/boss_watch.rs`
- `boss-watch-under-load-20261016T114846Z.log`

_Written incrementally; sections are appended as they are measured._

## 1. What the failing test actually executes (C-read)

- `boss_watch_reaches_detect_after_resume` (`tests/boss_watch.rs:659`) builds its engine with **`Engine::new`**
  (`:715-726`), **not** `engine_with_stub` (`:120`).
  - Every other execution test in the file injects `StubRunner`. This one alone gets the production
    `CargoXtaskRunner`.
  - Its `check` node therefore runs `/bin/sh -c "cargo xtask silent-noop-check"` (`heal.rs:45-53`). The cwd is the
    scratch workspace under `_tmp/`, so cargo walks up to the repo's (or the export's) `Cargo.toml`.
  - `xtask = "run -p xtask --"` (`.cargo/config.toml`), so the node's duration is **a nested cargo build of
    xtask, plus cargo's lock wait, plus the scan**.
- `silent-noop-check` itself is a source scan of four lists (`xtask/src/silent_noop/mod.rs:1-20`), so the scan is
  not where the time goes.
- **The production runner has a fixed 2-minute timeout per attempt** (`heal.rs:53`, `Duration::from_mins(2)`).
  - A timeout yields `pass = false` (`:60`). `HealDetectNode::run` then re-runs the gate once (`:133`).
  - If the second attempt also times out, the node **writes a `Detected` heal-ledger row and routes `detected`**
    (`:140-163`).
  - So under load, a timeout is recorded as a *reproducible gate failure*. In the real graph that escalates to the
    human gate (`boss-watch.toml`, `check → cyc → escalate`). **This is the defect hypothesis 2 names, and it is
    present.**
- **`NodeStarted(check)` is not visible while the node runs.** The journal is write-behind: events wait in a
  pending buffer until 16 accumulate or a barrier flushes (`pii-graph/src/journal.rs:4-26, :45`).
  - `wait_for_journal_len` polls `store.read_journal` (`:229`), which sees only flushed events.
  - So the start event the engine **already emits** is invisible to every reader for exactly as long as the node
    is busy.
  - The silence is not "no event exists". It is "the event exists and is held back".

## 2. Measurements

| # | what | result | label |
|---|---|---|---|
| M1 | `cargo xtask silent-noop-check`, cold, scratch `CARGO_TARGET_DIR`, idle host (load 1.04) | 51 crates compiled, `Finished … in 1m 13s` | C-run |
| M2 | the same, warm | 9.8 s wall | C-run |
| M3 | the same, cold, `CARGO_BUILD_JOBS=2` (a stand-in for a host with 2 free cores) | **102.8 s**, close to the runner's 120 s timeout | C-run |
| M4 | **run 1**: the failing test alone, in a scratch export of HEAD `1d0066f`, no xtask built yet (load 9-13, mostly my own preceding build) | **FAILED**: "last journal len 5, no growth for 60.008 s". `/bin/sh -c cargo xtask silent-noop-check` was alive at t = 100 s. | C-run |
| M5 | where did the nested build go? | **Not** into the `CARGO_TARGET_DIR` I passed (`tgt/debug/xtask` absent). It went into **`<export>/target/debug/xtask`**, finished 12:32:05.87, which is 1.4 s before run 1's test process ended. | C-run |
| M6 | **run 2**: the same test, immediately after, with `<export>/target` now holding xtask | **PASSED in 0.99 s** | C-run |
| M7 | **run 3, the idle control**: `<export>/target` moved aside (my scratch, `mv`), so xtask is cold again; 1-min load 2.29 at start, with no build running beyond the test's own nested one | **FAILED** the same way: 5 entries, 60.005 s. Test time 74.5 s. xtask finished building at 12:35:47. | C-run |
| M8 | why the nested cargo ignores `CARGO_TARGET_DIR` | `ProcCap` spawns with `env_clear()` and keeps only `PATH HOME LANG LC_ALL TERM PWD TMPDIR` plus `PII_*` (`pii-core/src/childproc.rs:53, :213`). `CARGO_TARGET_DIR` is dropped, so cargo uses `<workspace root>/target`. | C-read |
| M9 | history | 26 passes and 4 failures in the loop's logs. Passing runs took ~11 s for the whole `boss_watch` binary (10-11 to 10-14). First failure: `gate155-110442.log` (10-15 11:04). Then 10-16 06:38, 10:28 and 11:48. | C-run |

**What M4-M8 establish.** The stall reproduces **without** external load (M7). At the same moderate load it disappears the moment
`<export>/target` holds xtask (M6). The trigger is which target directory the nested cargo uses, not CPU contention.

**Why it began on 10-15 (inferred from M8 and the commit history).** Until 10-15, `fresh_gate.rs` pinned the gate's
`CARGO_TARGET_DIR` to `<export>/target`. That is exactly the directory the env-scrubbed nested cargo falls back
to, and earlier gate steps had already built xtask there, so the node took about 1 s.
- `bb217be` and `ca644a4` (10-15 12:20-12:29, `FRESH_GATE_TARGET` → `_tmp/gate-target`) moved the gate's builds
  elsewhere. `<export>/target` is now empty in every fresh export, and every gate pays a cold xtask build inside
  this one test.
- The 11:04 failure precedes those commits. It is a hand-run `quality-gate.sh` (its log starts at `==> fmt`), run
  under a shell whose `CARGO_TARGET_DIR` pointed away from the tree's `target/` (`1f1f691`, 01:43). Same mechanism;
  this one I did not re-run.
- **062's warm-gate repair is what exposed this.** It made the gate's target warm and the nested one cold.

## Verdict

The stall is **fully explained by the node's duration plus the silence**: there is no async or wakeup defect. The
duration is not "a gate takes 8-25 minutes". It is a **cold nested build of xtask, 73 s idle and 103 s with 2
cores**, which happens because the process capability scrubs `CARGO_TARGET_DIR`. Three real defects sit along that
path:

1. **D1:** the production runner silently builds a second copy of xtask in a target the gate does not warm (M5, M8).
2. **D2:** a runner timeout is recorded as a **reproducible gate failure**, with a heal-ledger row and escalation
   to the human gate (`heal.rs:53, :60, :140-163`).
3. **D3:** write-behind hides `NodeStarted` for as long as the node runs (`journal.rs:46`).

Load only moves the cold build from 73 s to 100 s or more. The 60 s stall bound is crossed either way.

## Q4. Is there a runtime defect at all?

**There is no scheduler or async defect.** The defect record's hypotheses 1 and 3 are eliminated by measurement:

| hypothesis | verdict | evidence |
|---|---|---|
| 1. blocking work inside an async task | **eliminated** | Runs and resumes execute on `spawn_blocking` (`engine/runs.rs:71, :327`, C-read). The same binary at the same load passes in 0.99 s once xtask is warm (M6). A starved runtime would not care where xtask lives. |
| 3. lost wakeup | **eliminated** | The test does not wait on a notification. It polls `read_journal` every 10 ms (`boss_watch.rs:226-252`). The run completes as soon as the nested build finishes (M5 → M6). |
| 2. a fixed internal timeout that gives up without saying why | **present, in the runner rather than the watch** | `Duration::from_mins(2)` per attempt, `timed_out` → `pass = false` → one retry → `Detected` row (D2). It did not fire in M4/M7, because the test's 60 s stall bound fires first. Under the 32-hog control, where the cold build is > 100 s (M3), it is next in line. |

**Change.** Two parts:
- `GateResult` gains `timed_out`. `HealDetectNode` routes a timed-out attempt to a new `inconclusive` port and
  writes **no** ledger row.
- The runner sets `CARGO_TARGET_DIR` explicitly through the `env` extras `ProcCap::run` already takes, or the
  node stops running cargo at all (Q2).

**Proof:**
- a scripted runner that returns `timed_out` twice → port `inconclusive`, and `heals.count()` is unchanged. Red on
  HEAD, which routes `detected` and writes one row.
- M7 repeated with the target fix → passes, and `<export>/target/debug/xtask` is never created.

## Q1. Must the engine require a start heartbeat and periodic progress from long nodes?

**Yes, but the engine must provide it, not each node.** A node running a blocking `ProcCap::run` cannot speak until
the child returns. And a heartbeat from the waiting thread only proves the thread exists, so it would make a wedged
child look healthy. The heartbeat has to carry **evidence of the child's work**.

**Change:**
- **R1:** `NodeStarted` is a flush barrier: one `flush()` after appending it, the same mechanism `Suspended` and
  `Finished` already use (`journal.rs:17-26`). The cost is one redb commit per node start.
- **R2:** while a node is in flight longer than `progress_every` (default 10 s), the run loop appends a
  `NodeProgress { node, elapsed_ms, child: { pid, cpu_ms, out_bytes } }` event and flushes it. The `child` fields
  come from `ProcCap`, which already owns the child handle and its captured streams (`childproc.rs`). `cpu_ms`
  comes from `/proc/<pid>/stat` of the process group.
- **R3:** a node with `Effects` including `PROC` and no `timeout_ms` is refused at validation. Every long node
  declares its own bound.

Rule for readers: **in flight with `cpu_ms` or `out_bytes` growing means working; in flight with both flat for the
bound means wedged.** That is the distinction the brief says cannot be made today.

**Proof:**
- In this test, `read_journal` shows `NodeStarted(check)` within 1 s of `spawn_resume`. Today it never appears
  before the node ends.
- With a cold xtask (M7's setup), at least 6 `NodeProgress` events appear with `cpu_ms` increasing.
- A fault-injected child (`sleep 600`) yields `NodeProgress` with `cpu_ms` flat, and the stall reader names it
  `wedged: check`.

## Q2. Should heal_detect run a gate inside the watch at all?

**No. It should record, and leave the gate to its own turn.**
- A gate is the most expensive and most contended operation on this host. It already has an owner with a lock,
  budgets, logs and a warm cache: the builder loop's `fresh-gate`.
- Running it again inside a daemon graph duplicates that cost (a second cold xtask, M5). It runs under a scrubbed
  environment that does not match the gate's, and with a 2-minute bound the gate itself cannot meet (8-25 minutes
  under load, asserted).
- In a deployed daemon there may be no `Cargo.toml` at the workspace at all (the unit's workspace is `$STATE_DIR`).
  `cargo xtask` would then fail at once, twice, and file a `Detected` row with an escalation every 30 minutes. This
  is inferred: boss-watch is not scheduled in the deployed config today.

**Change:** `heal_detect` reads the gate's **recorded verdict**: the last `fresh-gate` result file (pass/fail,
failing step, failing tests, `ts`). It routes on that:
- `ok` when the record is a PASS;
- `detected` when it is a FAIL reproduced in two consecutive records;
- `stale` when the newest record is older than N hours.

It never spawns a process. The `proc` cap leaves the graph, and the existing reproduce-before-record guard becomes
"two consecutive gate records agree".

**Proof:**
- `heal_detect` p99 duration < 50 ms over the boss_watch suite;
- `ps --ppid <daemon pid>` shows 0 `cargo` children across a day with boss-watch scheduled;
- a FAIL record followed by a PASS record → `flake`, with no ledger row;
- the graph validates with `caps = []`.

## Q3. A test driving a graph that contains a gate: bound on progress, or scale the deadline with load?

**Neither, for this test. It should not drive the gate.** Its stated purpose is the `timer → heal_detect` handoff
(`boss_watch.rs:652-657`), and it is the only test in the file that executes `heal_detect` without `engine_with_stub`.

**Change:**
- Build its engine with `engine_with_stub(&dir, vec![])`, as the other `heal_detect`-executing tests do.
- Where a test must exercise a real long node, bound it on **progress** using Q1's `NodeProgress` (child
  `cpu_ms`/`out_bytes` growing), with an absolute cap.
- Never scale a deadline with load. M7 failed at a 1-min load of 2.29, so a load-scaled deadline would have been
  generous on exactly the runs that hid the defect, and it would still have read "slow" rather than "cold build in
  a second target".

**Proof:**
- the test passes 10/10 with 32 pinned hogs and 10/10 idle, each in < 2 s;
- mutation: a stub runner that blocks forever makes it fail with a message naming `check` as in flight with no
  progress. A no-op change to the stub does not do that.

## Q5. The smallest change that would have made this visible on day one

**Make `NodeStarted` a flush barrier (Q1 R1)**, which is a single `flush()` call. Then have the stall message print
the **last journal event** it saw.
- With that, the 10-15 11:04 failure would have read: "no growth for 60 s; last event `NodeStarted(check)
  kind=heal_detect` 60 s ago".
- That points straight at the one node that shells cargo, instead of at "5 entries" and a load theory.
- A cheaper version still is to log `ProcCap` child argv, pid, cwd and its exit or timeout as observe events.
  `/bin/sh -c cargo xtask silent-noop-check` in cwd `_tmp/phase4.7/…` would have been in the gate output.

**Proof:**
- re-run M7 with only the barrier and the message change: the failure text contains `NodeStarted` and `check`;
- on HEAD it contains neither.

## Ranking (by effect)

| rank | change | proves it |
|---|---|---|
| 1 | The failing test uses `engine_with_stub` (Q3) | 10/10 under 32 hogs and 10/10 idle, each < 2 s |
| 2 | A timeout is `inconclusive`, never `detected` (D2) | scripted double timeout → no ledger row; red on HEAD |
| 3 | `NodeStarted` flush barrier, and the stall message names the last event (Q5) | M7 failure text names `check` |
| 4 | `heal_detect` reads the gate's recorded verdict and spawns nothing (Q2) | 0 `cargo` children of the daemon per day; p99 < 50 ms |
| 5 | Engine `NodeProgress` with child cpu/output, plus a mandatory `timeout_ms` for `PROC` nodes (Q1) | ≥ 6 progress events with rising `cpu_ms` on a cold build; flat `cpu_ms` named "wedged" |

## NOT VERIFIED

| item | why |
|---|---|
| The 32-hog control with a warm xtask | not run: I did not want to saturate a shared host. M6 at load 8-13 is the nearest measurement. |
| What the 5 journal entries are | `read_journal` needs the test harness or a daemon; inferred from the write-behind rule |
| The cause of the 10-15 11:04 hand-gate failure | inferred from its log shape and `1f1f691`; not re-run |
| Whether cargo would block on the outer `cargo test`'s lock if the nested build shared its target | not reached: the scrub sends the nested build elsewhere |

Scratch: `/projects/_tmp/rev064-4TDe/` holds the export, the `tgt/` and `target*` builds, and `run{1,2,3}.out`. It
is mine from this run and nothing was deleted (the warm target was moved aside, not removed). The repo is unchanged
except this file. Nothing was committed.
