# 067: 41 seconds of implementation, 105 minutes of gate. The boundaries, the caches, the instrument (Opus, 2026-10-17)

This is a read-only review. The only repo file written is this one, and nothing was committed. Evidence labels:
- **C-run**: I ran it today.
- **C-read**: I read the lines.
- **asserted**: from the brief.

Inputs: 064, 065, 066, `gate-steps.txt`, `quality-gate.sh`, `scripts/gate-lib.sh`, and today's
`_tmp/autonomy/logs/cycle-20261017T*.log` (46 files).

_Written incrementally._

## 0. The gate as it is (C-read, `gate-steps.txt`)

There are **14 steps**:
- 1 format step;
- **6 builds, each with its own target directory** under `$FRESH_GATE_TARGET` (`clippy-gate`, `hal-gate`,
  `build-gate`, `test-gate`, `doctest-gate`, and the coverage step's own);
- 7 `cargo run -q -p xtask -- …` source scans.

The brief's "eight" counts the format step plus the build-shaped steps up to coverage. The scans come after. Every
step runs **even after an earlier failure** (`gate-steps.txt` header), so a `fmt` failure at step one still buys
all six builds.

## 1. Measured today (C-run unless marked)

**Per-step wall clock.** I took the last `HEARTBEAT … elapsed=` line per step. `run_step` prints one every 30 s
(`gate-lib.sh`), so "< 30 s" means no heartbeat was printed. The sample is the 11 cycles today that reached the
gate.

| step | warm or incremental (median, range) | cold (05:26 cycle) | notes |
|---|---|---|---|
| fmt | **1.4 s** (timed by hand) | — | **FAILED in 8 of 11 cycles** |
| clippy (workspace) | < 30 s to 86 s | 510 s | |
| clippy (hal-gpiod) | ~3 s | 60 s | |
| clippy (llamacpp) | — | 720 s | it ran this morning and is no longer in `gate-steps.txt` |
| build | < 30 s to 79 s | 1,620 s | |
| test (lib/bins/tests) | **median 540 s** (450-960) | 1,590 s | an item touching `pii-server` recompiles 4 crates (pii-server, pii-cli, pii-mcp, xtask) and **relinks 35 + 15 integration-test binaries**: `Finished … in 7m 44s` |
| test (doc) | < 30 s to 90 s | 600 s | |
| **coverage-floor** | **median 1,020 s** (960-1,262) | 1,050 s | **never warm.** `cargo llvm-cov` recompiles **all 24 workspace members** on every run (24 `Compiling` lines, `Finished … in 15m 11s`). It cleans workspace artefacts by default; `--no-clean` is not passed. |
| 7 xtask scans | < 30 s each | — | |

So a **warm gate costs 16-32 min**, and coverage is half or more of it. **A cold gate costs about 102 min**, which is
the 05:26-07:12 cycle: 1 h 46 min wall.

**Four structural findings:**
1. **The gate fails late by design.** "Every step runs even after an earlier failure" (`gate-steps.txt`). The 8
   cycles whose `fmt` failed at 1.4 s **still ran 236 min of gate** (sum of those cycles' step times).
2. **The `fmt` defect was a hand commit with no format check on it.** `19024ce` (10-17 04:32) committed
   `xtask/src/fresh_gate.rs` unformatted, and `3ada006` (12:18) fixed it. **That is 7 h 46 min of red at step one
   for every item**, and nothing ran `cargo fmt --check` at the commit (C-run, `git log -L`).
3. **One item was gated eight times.** 155 is the item of 8 of the 11 gated cycles. Its resumed worker session
   made no change in at least one of them (05:26: "tree after: 1 dirty path: `__pycache__/`").
4. **The warm cache produced false failures.** `driver_spawns_no_subprocess`, `launcher_dials_through_the_uds_client`
   and `no_raw_dial_remains_in_pii_evolve` failed in the test step of the 12:00, 12:53 and 13:29 cycles with
   `read driver.rs: NotFound`. `crates/pii-evolve/src/driver.rs` exists.
   - The test binary `_tmp/gate-target/test-gate/debug/deps/launcher-b20b8ddc02b01e72` was built at 10:13 inside
     the export `/projects/_tmp/fresh-gate-1255751`. It embeds that path as `CARGO_MANIFEST_DIR`, and the export
     is gone.
   - The main-tree gate reused the binary, because cargo hashes workspace members independently of their absolute
     root, and the sources' mtimes were older than the binary.
   - **`coverage-floor` PASSES in the same three runs.** llvm-cov rebuilds the workspace members, and it exits
     non-zero on any failing test, so the three pass there. Its per-test output goes to `/dev/null`, so this is
     inferred from the step's verdict, not read from test lines. **Sharing one cache between two source roots
     manufactures failures that no item can fix.**

**Disk:** `_tmp/gate-target` holds 51 GB in total:
- **22 G in `debug/`**, which no step in `gate-steps.txt` names, so it is orphaned by an earlier layout;
- 20 G `test-gate`, 3.9 G `build-gate`, 2.9 G `doctest-gate`, 2.0 G `clippy-gate`;
- **536 M `llamacpp-gate`**, also orphaned, since its step is gone.

`target/llvm-cov-target` holds another 19 G. `/projects` is 89% full, with 64 G free.

## Q1. The split: item, commit, release

The rule: **each boundary proves what can change at that boundary, and nothing that is already proven or cannot
yet be proven.** An item changes a few crates. A commit changes master. A release changes production.

| boundary | runs | measured or derived cost | fails |
|---|---|---|---|
| **ITEM** (per attempt, inside the cycle) | `cargo fmt --check` · the item's own criteria · `cargo clippy -p <touched crates> --all-targets -D warnings` · `cargo test -p <touched crates>` · the 7 xtask scans (source-only) | fmt 1.4 s; the scans < 30 s each; clippy on the touched crates is within the 86 s workspace-incremental figure; tests on touched crates only, a fraction of the 540 s workspace median. **Target ≤ 5 min warm.** | **fast**: stop at the first red step |
| **COMMIT** (once per commit to master, after it lands; batched if several land within an hour) | workspace `clippy --all-targets`, `build`, `test --lib --bins --tests`, `test --doc`, and the scans | the warm-gate figures minus coverage: **about 10-14 min** (clippy ≤ 86 s + build ≤ 79 s + test 450-960 s + doc ≤ 90 s) | late is fine; its verdict goes to the ledger (Q5) |
| **RELEASE** (before `install-pii.sh`, and nightly on master if HEAD moved) | `coverage-floor` · `clippy (hal-gpiod)` · any feature-gated clippy (llamacpp) · `cargo build --release` (fat LTO) · `deploy-dry-run.sh` | coverage 960-1,262 s; hal 3-60 s; llamacpp 720 s cold; the release build is not measured today. **About 20-35 min**, once a day at most | blocks the deploy card |

**Also at the commit boundary: a `pre-commit` hook in the repo (`core.hooksPath`) running `cargo fmt --check`**
(1.4 s), for every committer: the loop, the orchestrator, and a human. `19024ce` was a hand commit. The gate was
the only format check, and it ran 7 h 46 min later on someone else's item.

**What "the touched crates" means, mechanically:**
```
git diff --name-only <base>.. | sed -n 's#^crates/\([^/]*\)/.*#\1#p' | sort -u
```
This is reverse-dependency-free on purpose. A `pii-server` change does **not** retest pii-cli at the item
boundary. The commit boundary does, within the hour.

**The trade, stated:** master can go red at the commit boundary for an item that was green at its own boundary.
The rule that makes this safe is the base-red stop (Q5): **a red commit-boundary verdict on master stops dispatch,
and the next item is the fix.** Today the opposite happens: red master is paid for by every item.

**Proof:**
- over the next 5 items, the item boundary's wall time p90 is ≤ 5 min, read from the ledger (Q5);
- each commit to master gets exactly one commit-boundary verdict within 60 min;
- coverage runs ≤ 1 time per day;
- a commit with a formatting error is refused by the hook. Fixture: re-apply `19024ce`'s unformatted hunk to a
  scratch clone and `git commit` → rc ≠ 0.

## Q2. `coverage-floor`: who pays, when, and what the item boundary does instead

- **Who pays: the release boundary, and one nightly run on master when HEAD moved.** At a median of 1,020 s, never
  warm (all 24 members are recompiled), it is the most expensive step and the least item-specific. Its question is
  "did the tree's coverage fall below a floor", not "is this change right".
- **When:** nightly (one run per day, so 17 min per day instead of 17 min per attempt; today it ran 10 times, about
  170 min), and as a hard precondition of the deploy card.
  - A nightly failure names the commits since the last passing verdict. The bisect window is ≤ 1 day.
  - The loop's next item is the fix, by the base-red rule.
- **Make the nightly run cheaper too:** pass `--no-clean` to `cargo llvm-cov`, so the instrumented build is
  incremental, and keep `cargo llvm-cov clean --workspace` for a weekly full run. The clean exists to avoid stale
  profiles, and a weekly clean bounds that risk.
- **The item boundary instead:** no coverage measurement at all. It asks one cheap structural question, **"does the
  diff that changes `src/` also change or add a test?"** (a `git diff --name-only` check), unless the queue block
  (066) declares the item test-only or docs-only. This is a proxy, not a measurement, and the nightly run is the
  measurement.

**Proof:**
- `coverage-floor` runs ≤ 1 per day in the ledger;
- with `--no-clean`, the second nightly run on a one-crate change shows ≤ 3 `Compiling` lines (today: 24);
- an item whose diff touches `crates/*/src` with no `tests/` or `#[test]` change is refused at the item boundary.
  Fixture: a src-only diff → refused; the same diff plus a test → accepted.

## Q3. Caches: per step, warm, pruned. The rule that makes a cold compile impossible

"Warm" has been an aspiration enforced by nothing. Today it failed three ways:
- a new directory starts empty (the cold 05:26 cycle);
- coverage cleans itself;
- one directory is shared by two source roots, which breaks correctness, not just speed (§1 finding 4).

**The rule:** a build step may run only against a cache that **declares** it is warm for **this** source root
and toolchain. Otherwise the step refuses, and a separate, explicit warm job builds it. A cold compile can then
happen only in a job whose name says `warm`, never inside an item or commit gate.

**Change:**
- **R1. Key the cache by source root.**
  - Layout: `_tmp/gate-target/<sha8(realpath of source root)>/<step>/`.
  - The export path is fixed (062 rank 2), so the main tree and the export have one key each and never share a
    directory.
  - Binaries embedding `CARGO_MANIFEST_DIR` can then never outlive their root.
- **R2. Stamp and check.**
  - Each cache directory holds a `.gate-cache` stamp: `{root, rustc -vV hash, Cargo.lock sha, last_warm_ts}`.
  - `run_step` checks it before the step runs. A missing stamp, another root, or another rustc means
    `FAIL: <step> COLD CACHE (<reason>)` in under 1 s, with **no build**.
  - A changed `Cargo.lock` only marks the stamp `stale`: the step runs, and the warm job is queued.
- **R3. Warm explicitly.** `quality-gate.sh --warm <boundary>` builds each step's cache and writes the stamp.
  - It runs at the commit boundary, after a `Cargo.lock` or toolchain change, and nightly before the release run.
  - It is the only code path allowed to compile from cold.
- **R4. Share where cargo allows it.** `clippy`, `build` and `test` can share one directory, because cargo keys
  units by profile and features. Keep separate directories only where a measured reason exists:
  - `doctest` (F-01, E0463);
  - `coverage` (instrumented);
  - `hal` (a feature build).
  That turns 6 directories into 4, and the dependency graph is no longer duplicated in `build-gate`,
  `clippy-gate` and `test-gate`.
- **R5. Prune by name, never by guess.**
  - A directory under `_tmp/gate-target/<key>/` that no line of `gate-steps.txt` names is **ORPHAN**. Today that
    is `debug/` (22 G) and `llamacpp-gate/` (536 M).
  - `ctl.sh status` and `health.sh` print orphans with their sizes.
  - `quality-gate.sh --prune` moves orphans to `_tmp/gate-target.orphan-<ts>/`, per the no-delete rule, and the
    operator removes them.
  - A per-key budget (e.g. 40 G) is reported when exceeded, never silently evicted.

**Tests that pin it** (`scripts/checks/gate-cache-selftest.sh`, called from `selftest.sh`, all on a tiny two-crate
fixture workspace, so the test costs seconds, not a gate):
1. **"Warm means zero":** build step, then the same step again on an unchanged tree.
   `grep -c '^ *Compiling' == 0` on the second run. This is the regression pin: any change that makes the cache
   miss turns it red.
2. **"Cold refuses":** run the step against an empty directory → `FAIL … COLD CACHE (no stamp)`, rc ≠ 0, wall
   < 2 s, and 0 `Compiling` lines.
3. **"Another root refuses":** warm from root A, then run from root B against A's directory → `COLD CACHE (root
   mismatch)`. **Today's failure, as a fixture:** a test using `env!("CARGO_MANIFEST_DIR")` built from A, with A
   moved away. On HEAD it fails `NotFound` in the reused binary, as measured in §1; with R1-R2 it is refused
   before it runs.
4. **"Orphans are named":** create `_tmp/gate-target/<key>/stray/` → `status` lists it under ORPHAN with its size,
   and `--prune` moves it (the original path is gone, and the dated path exists).
5. **Mutation:** disable the stamp check. Tests 2 and 3 must go red and test 1 stays green.

## Q4. What 41 s against 105 min says about the design, and the smallest change to minutes

**The ratio is 0.65%.** The loop's unit of verification is the most expensive proof the repo has, release-grade
CI, bought **per attempt, serially, failing late, and again on every retry of the same item** (155: 8 gates
today). It answers "is the whole tree release-ready?" every time it wants to know "is this change right?".

The design consequence is visible in today's numbers:
- **items cannot be cheaper than the tree's worst step** (coverage, 17 min, never warm);
- **items inherit the tree's defects:** `fmt` from `19024ce`, and the stale-cache NotFound. Both are base-red
  failures no item could fix, and every item paid for them.

**The smallest change, two edits:**
1. **A boundary column in `gate-steps.txt`** (`item|commit|release`), plus
   `quality-gate.sh --boundary item --fail-fast`.
   - The cycle calls the item boundary.
   - Fail-fast alone would have turned the 8 `fmt`-red cycles from 236 min of gate into 8 × 1.4 s.
2. **The base-red stop (Q5):** before dispatching, the tick reads the last commit-boundary verdict for HEAD. If it
   is red, it dispatches nothing and says `BASE RED <step> since <rev>`.

The commit-boundary run is the existing gate, minus coverage, triggered after a commit lands, under the same lock.
Coverage moves to nightly (Q2). Nothing new is built: no daemon, and no change to any budget or criterion.

**Proof:**
- dispatch-to-commit p90 ≤ 15 min over the next 5 items (today about 105 min for the cycle the brief quotes);
- the gate's share of cycle wall time falls from > 99% to < 70%;
- master's commit-boundary verdict is green on ≥ 80% of commits;
- a fixture with a red `fmt` on HEAD makes the tick print `BASE RED fmt` and dispatch nothing (rc 0, no worker
  process).

## Q5. The instrument that would have shown this on day one

**The gate ledger:** `_tmp/gate/ledger.jsonl`, written by `run_step`, **one line per step per gate run**:

```json
{"ts":"…","run":"<gate run id>","boundary":"item","rev":"<HEAD>","base_rev":"<merge base>","item":"A22","attempt":5,
 "step":"coverage-floor","wall_s":1020,"compiled":24,"cache":"_tmp/gate-target/<key>/coverage","cache_bytes":…,
 "rc":0,"failing_tests":[]}
```

It is the per-step form of 065 Q2's `verdicts.jsonl`. `compiled` is the count of `^ *Compiling` lines in the step's
log, which `run_step` already captures to `target/gate-steps/<label>.log`.

**Three derived alarms**, printed by `tick.sh` through `say_once`. Each is one `jq` over the ledger:

| alarm | rule | today it would have fired |
|---|---|---|
| **BASE RED** | the same step fails with the same `failing_tests` (or the same first `Diff in` line, for `fmt`) on **≥ 2 distinct items**, or on the commit-boundary run of `rev` | **05:26**, on the second `fmt` failure after `19024ce`, instead of the hand fix at 12:18; and **12:53**, for the three pii-evolve NotFound tests, which are still failing at 13:29 |
| **COLD** | a step on a warm-stamped cache shows `compiled > 3` with an unchanged `Cargo.lock` and toolchain | **every coverage run** (24 compiled), so on the first night; and the 05:26 cycle's clippy, build and test steps |
| **COST SHARE** | per cycle, `sum(wall_s of gate) / (cycle wall)` > 0.9, or one step's 7-day median > 300 s | coverage's median of 1,020 s, and the ratio the brief measured by hand (105 min against 41 s) |

**Why this instrument and not a dashboard:** every number in this document had to be reconstructed by parsing
30-second heartbeats out of 46 logs. With the ledger, each is one `jq` query, and the tick asks those queries
every 30 minutes.

**Proof:**
- after one gate run, the ledger has exactly 14 lines for that run (one per step), with `wall_s` summing to the
  run's wall time within 5%;
- **replay test:** feed today's 11 cycle logs, converted to ledger lines by a one-off parser, to the alarm queries.
  BASE RED fires for `fmt` at 05:26 and for the pii-evolve set at 12:53, COLD fires for every `coverage-floor`
  line, and none fires on the 00:33 cycle's healthy steps;
- **mutation:** make `run_step` skip the ledger write. The tick's alarm check must then print
  `ledger stale (no line since <ts>)`, not silence.

## Ranking (by effect, cheapest first where equal)

| rank | change | proof | red today |
|---|---|---|---|
| 1 | **Fail-fast at the item boundary, plus a `pre-commit` `cargo fmt --check` hook** | 8 × 1.4 s instead of 236 min; `19024ce`'s hunk is refused at commit | every step runs after a failure; no hook |
| 2 | **Key caches by source root, with stamps, refuse-cold, and an explicit warm job** (Q3) | warm re-run: 0 `Compiling`; root-mismatch fixture refused; the NotFound class is gone | 3 tests fail in 3 cycles on a binary from a dead export |
| 3 | **The gate ledger and its three alarms** (Q5) | the replay fires BASE RED at 05:26 and 12:53 | per-step cost exists only in heartbeats |
| 4 | **The boundary column: item / commit / release** (Q1), with the base-red stop | item p90 ≤ 5 min; cycle p90 ≤ 15 min | a full gate per attempt |
| 5 | **Coverage nightly and at release only, with `--no-clean`** (Q2) | ≤ 1 coverage run per day; ≤ 3 compiled on a one-crate change | 10 runs today, 24 compiled each |

Pruning the orphans (`debug/` 22 G, `llamacpp-gate/` 536 M) is the operator's call under the no-delete rule. It
frees about 22.5 G at once.

## NOT VERIFIED

| item | why |
|---|---|
| The brief's "41 s" worker time | the cycle logs carry no per-line timestamps; I did not open the worker transcripts |
| Item-boundary cost of ≤ 5 min | derived from the incremental figures, not run, because a gate run would load the host the loop is using |
| The release build's wall time | not run today |
| That the three pii-evolve tests pass under coverage | inferred from `coverage-floor` PASS; its per-test output goes to `/dev/null` |
| That clippy, build and test can share one directory without reintroducing F-01 | F-01 concerned doctests, which stay separate; not tested |

Nothing in the repo changed except this file, and nothing was committed. The three modified `crates/pii-server`
files in `git status` belong to the loop's in-flight item, not to this review. No scratch was created.
