# 058 — The cost findings: RSS growth, write amplification, the observer A/B (spec 116)

**Date**: 2026-10-10. Closes spec 110's F2, F3, F8. Debug build, scripted model, shared host (load average
~10-15 throughout), one persistent client connection unless stated. Every number below was produced by
`scripts/archive/leak_bisect.py` or by the named test; the raw per-poll series are in
`/projects/_tmp/leak-bisect/*/state.json` (scratch, not committed).

## R1 — F2: where the RSS growth comes from

**Harness** (committed, re-runnable): `python3 scripts/archive/leak_bisect.py --runs N --every K --variants a,b [--graph G]
[--parallel] [--keep|--resume]`. Fresh store and daemon per variant, one run = `run.start` + poll `run.get`,
samples `/proc/<pid>/status` (`VmRSS`, `RssAnon`, `RssFile`), `sys.status` store bytes, `/proc/<pid>/io`, CPU.
Variants: `base obs-off audit-off both-off traj-on conn-per-run idle-conn`. Two runs of
`--runs 4000 --every 1000 --variants base` (lean graph, same binary):

    run 1: write_bytes_per_run=51236  wchar_per_run=47370  rss 21076 -> 26104 kB  slope(2nd half)=946 KB/1000
    run 2: write_bytes_per_run=51232  wchar_per_run=47364  rss 22040 -> 27192 kB  slope(2nd half)=1080 KB/1000

Bytes reproduce to 0.01%; RSS slope to ~12% (page-granular, and the host is shared).

**Bisect — RSS (kB) vs runs, lean graph (1 transform node), binary BEFORE the fixes, fresh store, 32 000 runs:**

    runs     base    obs-off(c)  conn-per-run(e)  idle-conn(e, no runs)
    0       21344      21264        21308            21200
    4000    26632      26696        26856            21620
    8000    31176      31176        31360            21856
    16000   33804      33864        34216            21892
    24000   34236      34280        34460            21928
    32000   34276      34412        34628             -      (idle-conn: 24 000 connections)
    B/run   0-8k: 1258 | 8-16k: 336 | 16-24k: 55 | 24-32k: 5      (base)
            1269 | 344 | 53 | 17                                  (obs-off)
            1287 | 366 | 31 | 22                                  (conn-per-run)

At 32 000 runs `RssFile` is a constant ~20 MB (the binary) and all growth is `RssAnon`
(4.3 -> 14.3 MB). **It plateaus.**

**Agent graph (spec 110's workload: 1 scripted turn, 8 events), BEFORE the fixes:**

    runs     base    obs-off   traj-on
    8000    42264    41056     43592
    16000   42692    42384     47216
    24000   42976    42728     49444
    32000   43320    43092     53144
    B/run (22k-32k)  36        39        373

| component | verdict | evidence |
|---|---|---|
| (c) observer fan-out | **ruled out** | obs-off tracks base within 0.2 MB at every point; `write_bytes` identical to 8 digits |
| audit feed | **ruled out** | audit-off: 209 174 528 vs 209 182 720 bytes written over 4 000 runs (the 279-byte file) |
| (e) client connection handling | **ruled out** | conn-per-run == base (34.6 vs 34.3 MB at 32k); 24 000 idle connections: +0.7 MB total, ~4 B/connection after the first 2 000 |
| (a) engine run table / sessions | **ruled out by measurement of the remainder** | the store-only probe (below), with NO engine, shows the same warm-up-then-plateau shape; `run.start` creates no session; `unregister_run` drops each handle. Not separately instrumented (no `runs_active` series was kept) |
| (b) journal write-behind / store (redb) | **where the warm-up and the residual live** | store-only probe: 7.8 -> 19.2 MB, flat after ~12 000 runs (below) |
| (d) trajectory sink (control, default OFF) | **a real leak, FIXED** | +300-370 B/run above base, unbounded; see below |
| the "no plateau" of spec 110 | **= warm-up + (d)** | spec 110's soak ran with trajectories ON and was read at 20-85k runs, but its 300 s baseline (8k runs) was inside the warm-up |

**Store-only probe** (`cargo test -p pii-graph --test rss_probe -- --ignored --nocapture`, no engine, no bus,
no observer, no connection; `store.next_run_id` + `run::run`, lean graph):

    runs   0     2000   4000   6000   8000   10000  12000  14000  16000  18000  20000
    kB     7820  10004  11856  14040  16264  18148  18580  18824  18972  19160  19160

**The residual, stated precisely.** After warm-up (RSS growth per run, B): lean graph 5-22 (24-32k), agent
graph 36-39 (22-32k), store-only probe 0-47 (16-20k). It is in the store/runner path (the store-only probe
has it), NOT in the observer, audit feed, connection handling or trajectory sink. It is NOT explained: the
suspects are redb's allocator/tracking state, which grows with the file (the store grows ~1.1-1.7 KB/run),
and allocator fragmentation. At the post-fix ceiling of ~100 runs/s, 40 B/run is ~14 MB/hour; at an
operator's rate it is negligible. **What it did not do:** attribute those last ~5-40 B/run to a line of
code; that needs a heap profiler (none on this host).

**The fix (d):** `TrajectorySink` kept one `RunFile` map entry per run it ever saw and `account()` scanned
the entire map on EVERY event to build the in-flight set — an unbounded map AND a per-event cost that grew
with every run (which is spec 110's "mild upward latency drift"). Finished runs are now removed
(`crates/pii-observe/src/trajectory.rs`); a run that hit its per-run cap keeps its entry so a late event
stays gated. Mutation control: `a_finished_run_is_released_by_the_sink` — RED before the fix
(`tracked_runs()` 1, expected 0), GREEN after. Sink-only probe
(`cargo test -p pii-observe --test rss_probe -- --ignored --nocapture`, 4 events/run, default 50 000-file
retention):

    BEFORE (release disabled):  tracked_runs == runs ever seen (10000, 20000 ... 60000); RSS 3.2 -> 22.5 MB at
                                60 000 runs and the probe was killed at 580 s (quadratic: 150 000 runs never finish)
    AFTER:                      tracked_runs == 0 throughout; 150 000 runs in 22 s; RSS 3.2 MB -> 12.5 MB at 50 000
                                (the retention index, ~190 B per retained file, bounded by trajectory_max_files),
                                then 16.2-19.0 MB flat to 150 000

Daemon, trajectories ON, agent graph, after the fix: 60 000 runs, 50 001 files on disk (the retention bound
held), RSS 34.0 -> 62.4 MB at 60 000 (spec 110: 60.5 -> 86.4 at 85 018). The daemon series still climbs at
~270 B/run in this window; the sink-only probe says that is the index filling toward the 50 000-file bound
plus allocator steps, but **the daemon was not run past ~90 000 runs to watch it flatten** (a second
daemon series failed to resume — the first chunk had been stopped — and was not repeated).

## R2 — the leak is visible

New gauge `pii_rss_bytes_per_run` (`pii metrics`; `sys.status.rss_bytes_per_run`): RSS growth per run
between the oldest of up to 32 checkpoints (one per >= 1000 runs, on status polls) and now; 0 until
4000 runs span the window. The runbook (`_docs/05-operations/105-runbook.md`, "Memory growth") states the
expected curve (warm-up ~1.3 KB/run on the lean graph and ~2.5 KB/run on the 8-event agent graph, for ~20 000 runs; then under ~100 B/run; read the gauge only after
~60 000 runs) and what is not expected. Tests: `rss_trend` unit tests (steady leak reads as its slope; flat
and shrinking read 0; short window reads 0; bounded), `rss_gauge_reports_growth_per_run_and_zero_when_flat`
(mutation: removing the `note` call fails it), and `health_identity` (field present on the wire).
**Not covered by a unit test:** that `identity::attach` calls `rss_gauge` with the live RSS — the wiring is
one line, checked by `status_keeps_old_keys...` only for presence; I did not read the gauge live above 4 000
runs in a daemon.

## R3 — F3: the ~50x write amplification

Measured: the amplification was **not the events**. One redb commit costs **24 576 B** whatever it carries
(6 x 4 KiB pages: the table tree, the freed-page tree, the header); a run made **four** commits — the id
counter, the graph body, the input body (both content-addressed and identical to the previous run's), and the
journal append. Per run, 4 000 runs in, default config:

    writer                                  before        after
    run-id counter commit                   ~24 600 B      766 B   (ids reserved 32 at a time)
    graph body commit                       ~24 600 B        0     (already stored: no commit)
    run input body commit                   ~24 600 B        0     (same)
    journal + header + status (1 commit)    ~50 400 B     50 429 B
    audit.log                               0 per run (279 B once)
    log file / trajectory (opt-in)          ~205 B / +4 085 B
    DAEMON TOTAL (lean, leak_bisect.py)    124 150 B      51 236 B   (-58.7%)
    in-process sum (write_amp test)                       51 195 B   (766 + 50 429; 0.08% from the daemon)

**Tolerance stated: the parts must sum to the daemon total within 5%; they do, within 0.1%.** The per-table
split of the one remaining commit (`cargo test -p pii-store --test write_amp -- --ignored --nocapture`,
deterministic across reruns): fixed 24 548 B; journal rows +8 220; run header +8 929; run_by_status +8 219;
cumulative probe 49 916 B vs 45 274 B for the real `append_events` — **10% apart**, because it is 4 KiB-page
arithmetic. Read it as "each table costs about one tree path".

Agent graph, same fix: 126 706 B/run (8 000 runs) -> 52 296 B/run (4 000 runs); with the 24 000-run
series 58 986 B/run (the figure grows with the store: btree paths deepen — 120 -> 131 KB over 32k runs
before). Throughput on this host went from ~45 to ~80-120 runs/s: the disk was the ceiling.

**Reductions taken and why they do not weaken durability or the audit trail:**
1. `Store::next_run_id` reserves ids 32 at a time (one commit per block). A restart skips the rest of a
   block, so ids can have **gaps of up to 31 after a restart**; they are strictly increasing and never
   reused. (Test `run_ids_are_never_reused_across_a_reopen`; `run_ids_are_reserved_in_blocks...`.)
2. `Store::put_graph` no longer commits a body that is already stored (content-addressed, immutable).
   (Test `storing_a_body_that_is_already_stored_commits_nothing`; mutation: disabling the check fails it.)
3. Nothing about the journal append, the run header, the status index or `audit.log` changed. The
   remaining 50 KB is the one commit the run needs; no further reduction is available without batching
   runs' commits together, which WOULD weaken durability — not done.
`write_amp.rs` (pii-graph) asserts < 1.2 commits per run (it was 4); mutation: disabling the body check fails it.

## R4 — F8: the A/B spec 110 could not run

Switches: `pii daemon --observer on|off`, `--audit on|off`; `[core] observer = "on"|"off"`, `audit = ...`.
Flag > `[core]` > on. Default ON; anything but `on`/`off` is exit 2. Off is announced on stderr, as a warn
log event (`daemon.observer.detached`, `daemon.audit.disabled`) and in the audit trail; with audit off
`audit.log` keeps only `daemon.audit.disabled`. What `observer off` does: no `ToolCallObserver` on runs
(`Engine::detach_observers`), no `ContractObserver`, the late-bound log stays unattached. The bus,
trajectory and audit feed are unaffected.

A/B, agent graph, 3 000 runs per round, 3 rounds, sequential (so timing is valid), fresh store each:

    round   base cpu_ms/run  runs/s    obs-off cpu_ms/run  runs/s    both-off cpu_ms/run  runs/s
    1       10.05            111.4     12.53               86.4      13.12                82.0
    2       13.05            83.0      12.96               83.3      12.75                84.3
    3       13.15            80.7      13.24               81.3      10.64                103.4
    mean    12.08            91.7      12.91               83.7      12.17                89.9

**Below resolution.** The baseline's own spread over three rounds is 10.05-13.15 ms/run (+-1.5) and
80.7-111.4 runs/s; every delta (+0.8 ms for observer off, +0.1 for both) is inside it. Bytes written per
run are identical to the byte (209 182 720 vs 209 182 720) and RSS slopes agree. Conclusion: with a scripted
model and the default config (no log file, trajectories off), **the observer fan-out and the audit feed cost
nothing this harness can see**. The 3-round, 3 000-run design cannot see a difference smaller than ~1.5 ms/run
(~12%); a log file or trajectory sink, which do real I/O, WERE visible in spec 110 (+205 and +4 072 B/run).

## R5 and the gate

New names in `pii_observe::names::ALL`: `daemon.observer.detached`, `daemon.audit.disabled`,
`pii_rss_bytes_per_run`. Documented in the runbook with default and risk: the two switches, the gauge, the
write-amplification table, the id gaps. `bash quality-gate.sh` exit 0 (coverage 79.36% >= 74.00%, 3 m 37 s);
`cargo run -q -p xtask -- fresh-gate --worktree` -> `fresh-gate: PASS` (4 m 18 s); clippy `-D warnings` and
`cargo fmt --check` exit 0.

## Files
Modified: `crates/pii-cli/src/{cmd_daemon,cmd_metrics,daemon_log}.rs`, `crates/pii-observe/src/{audit,lib,names,trajectory}.rs`,
`crates/pii-proto/src/sys.rs`, `crates/pii-server/src/{lib.rs,daemon/config.rs,daemon/identity.rs,engine/runs.rs}`,
`crates/pii-server/tests/{health_identity,log_wiring}.rs`, `crates/pii-store/{Cargo.toml,src/lib.rs}` (+`parking_lot`),
`examples/pii.production.toml`, `_docs/05-operations/105-runbook.md`, `Cargo.lock`.
New: `scripts/archive/leak_bisect.py`, `crates/pii-observe/src/rss_trend.rs`, `crates/pii-observe/tests/rss_probe.rs`,
`crates/pii-graph/tests/{write_amp,rss_probe}.rs`, `crates/pii-store/tests/write_amp.rs`, this report.
Config keys: `[core] observer`, `[core] audit`. Flags: `--observer`, `--audit`. Wire: `sys.status.rss_bytes_per_run`.

## What this did NOT measure
Release builds; real model latency or tools; more than one client connection in the bisect (spec 110 used
4-8); runs of days; the daemon past ~60 000 runs with trajectories on; the engine's in-memory run table as its
own variant (ruled out by the store-only probe and by reading, not by a switch); the last ~5-40 B/run
(unattributed); log-file and trajectory write costs (taken from spec 110 and one 4 000-run delta); a
production-shaped store (the deepest was ~90 000 runs / 120 MB); the Landlock-confined (`--sandbox workspace`)
path — every daemon here ran `--sandbox off`. Process note: the observer/audit switches, the id block and the
metric were written before their tests; the RED-before-GREEN order was observed only for the trajectory leak
and the body dedup. Every new surface has a mutation that fails its test (detach no-op, audit-off filter,
switch default, id block = 1, dedup off, trajectory release off, trend feed removed).
Scratch (`/projects/_tmp/leak-bisect`, ~1.0 GB incl. 50 000 trajectory files, and `/projects/_tmp/trash-116`)
is left in place for a human to remove; nothing pre-existing was deleted.
