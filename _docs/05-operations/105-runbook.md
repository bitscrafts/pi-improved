# Pii operator runbook

Short and operational. Paths assume the shipped systemd unit
(`deploy/pii.service`) and `examples/pii.production.toml`; adjust to yours.

## Where things live

| What | Default | Set by |
|---|---|---|
| Main store | `/var/lib/pii/store.redb` | `pii daemon --store` |
| Approval / heal / memory stores | next to the main store: `approval.redb`, `heal.redb`, `memory.redb` | follows `--store` |
| Channel / timer stores, `control.sock` | `<runtime_dir>`: `channel.redb`, `timer.redb` | `$PII_RUNTIME_DIR` or `[core].runtime_dir` |
| Config | `/etc/pii/pii.toml` | `--config` or `$PII_CONFIG` |
| Log file (JSON lines) | `/var/log/pii/pii.log` | see "Logs" |

## Start / stop

    systemctl start pii          # or: pii daemon --foreground --store ... (foreground)
    systemctl stop pii           # SIGTERM; the daemon exits 0 within 2 s
    PII_RUNTIME_DIR=/run/pii pii health --json    # probe: non-zero exit = not running

## Read health and metrics

    pii health            # human text: build, config hash, stores with format_version, mounts, counts
    pii health --json     # the full `sys.status` payload
    pii metrics           # one `name value` line per metric (below)

`pii health --json` lists `stores[]` (main, memory, approval, heal, channel);
each carries `path`, `bytes` and a real `format_version`. `timer.redb` is
stamped too but is not listed in the payload.

`pii metrics` names (all consts in `pii_observe::names`):

    pii_runs                           gauge    runs in the main store
    pii_sessions                       gauge    sessions in the main store
    pii_approvals_pending              gauge    approvals awaiting a decision
    pii_journal_events_total           counter  journal events appended
    pii_memory_docs                    gauge    memory documents
    pii_cron_entries                   gauge    configured cron entries
    pii_uptime_ms                      gauge    daemon uptime
    pii_process_rss_kb                 gauge    daemon resident set size (VmRSS, KiB)
    pii_rss_bytes_per_run              gauge    RSS growth per run over the last ~32 000 runs (0 until 4 000 runs span the window)
    pii_store_bytes_per_run            gauge    main-store bytes / runs: the growth rate as one number
    pii_store_bytes{store="main"}      gauge    file size, one line per store
    pii_store_format_version{store=…}  gauge    persisted format version, one line per store
    pii_worker_requests_refused_total  counter  worker requests refused/cancelled around a reload
    pii_run_latency_count{outcome=…}   counter  runs recorded, per outcome (succeeded|failed|suspended|refused)
    pii_run_latency_ms{outcome=…,quantile="0.5|0.95|0.99|1"}  gauge  run lifecycle (accepted -> terminal), ms

Quantile lines appear only for an outcome that has samples. `quantile="1"` is the exact maximum.
`pii metrics` also prints `# NOTE …` lines (the store note below).

### Loaded-system numbers: what the percentiles mean (spec 115 R4)

The latency numbers come from a bounded in-process histogram (150 fixed buckets, ratio 1.1, no
dependency, no lock, constant memory). **Resolution:** a percentile is the upper bound of the bucket
holding that rank, capped at the largest value seen — it is never under-reported and over-reported
by at most 10% (+1 ms at the low end). Durations are whole milliseconds, rounded up (a 0.2 ms run
reads 1 ms). **What it cannot see:** a run longer than about 24 minutes lands in the last bucket
(only `quantile="1"` is true for it); the histogram starts at daemon start and is not reset; a
percentile over fewer than ~100 runs is an order statistic, not a tail estimate; **a run that
suspends is recorded as `suspended` at the moment it suspends, not when it finally finishes**
(`run.resume` lifecycles are not in the histogram).

Checked against an outside measurement of the same workload (real daemon, UDS, 400 runs, 4
connections, scripted model, debug build, shared host): client wall clock from the `run.start`
reply to the terminal seen by `run.get` (5 ms poll), vs `pii metrics`:

    quantile   client (outside)   pii metrics   delta
    p50        74.2 ms            73 ms         1.2
    p95        117.5 ms           118 ms        0.5
    p99        130.7 ms           130 ms        0.7
    max        144.2 ms           144 ms        0.2

RSS, same two runs: `/proc/<pid>/status` VmRSS read just after vs `rss_kb` from `sys.status`:
26 560 vs 26 328 kB (0.9%) and 26 232 vs 26 192 kB (0.15%); RSS tolerance used: 2%.

**Tolerance used: |difference| <= 10% of the value + 10 ms** (the 10% is the histogram's own
resolution; the 10 ms is one 5 ms poll interval plus scheduler jitter on a loaded host). A second
run under heavier host load differed by up to 8.1 ms at p95 (143 vs 151 ms), inside that bound.
The client interval starts AFTER the `run.start` reply, so it is always a sub-interval of the
server's accepted -> terminal interval; measured from before `run.start` the client reads ~30 ms
higher (that is the `run.start` RPC itself, which the server histogram does not include).

### Graph metrics: the journal is the surface (spec 109 R3)

`graph.node.dead`, `graph.join.unsatisfiable` and `graph.batch.width` are NOT daemon-log
lines and NOT `pii metrics` lines: they fire per tick, so logging them would flood the log.
The run journal records the same facts, and `pii graph trace` reads it
(`DB` = the store file, `RUN` = the run id):

    pii graph trace RUN --db DB --max-batch-width          # largest graph.batch.width of the run
    pii graph trace RUN --db DB | grep '"NodeDead"'        # one line per graph.node.dead
    pii graph trace RUN --db DB | grep 'never be satisfied'  # the Finished event of an unsatisfiable join (graph.join.unsatisfiable);
                                                           # `--terminal` alone prints just `failure`

`crates/pii-graph/tests/journal_is_the_metric_surface.rs` pins that the journal and the metric
stream agree, so these queries are a faithful substitute for a live metric.

## Polling a run: `accepted` vs unknown (spec 115 R1)

`run.start` returns the id before the run executes. `run.get <id>` (and `pii run get <id>`) answers:

| Answer | Meaning |
|---|---|
| `status: accepted` | the daemon holds the run, its journal has not materialised yet — poll again |
| `running` / `suspended` / `succeeded` / `failed` | the stored state |
| `not found` ("unknown to this daemon") | the id was never issued, or the daemon restarted and lost an accepted-but-never-journalled run |

A start the run's own validation refuses (for example V2: a node needs a capability the graph does
not grant) still returns an id; that id goes to `failed` with the reason in `run.trace`, a
`run.start.refused` event is logged, and the latency histogram counts it under `refused`. It never
polls `not found` forever. Measured: 400 runs polled to terminal produced 4 173 `accepted` answers
and 0 `not found` (the 056 baseline was 10.3 `not found` per run).

## Trajectory retention (spec 115 R2)

Trajectories are off unless `trajectory_dir` is set. The directory is bounded in aggregate, oldest
finished run first, on top of the per-run caps (10 000 events / 16 MiB):

    [core]
    trajectory_max_bytes = 536870912   # default 512 MiB total
    trajectory_max_files = 50000       # default 50 000 files

A `0` means "use the default", never "unbounded". When a bound is exceeded the sink deletes oldest
finished trajectories down to 90% of the bound and logs `trajectory.pruned`
(`files`, `bytes`, `remaining_files`, `remaining_bytes`). A run that is still open is never pruned;
if only open runs remain and the bound is still exceeded the sink logs `trajectory.over_budget`
once per episode and keeps writing — the current run is never dropped silently. The index is built
by one scan of the directory the first time a file is created after a restart, so existing files
count. Measured (400 runs, `trajectory_max_files = 20`): 19 files / 43 548 B afterwards vs 400 files
/ 916 800 B with the bound out of the way; newest trajectory ends in its `run.outcome` line;
`trajectory.pruned` logged 127 times at that tiny budget (one per ~3 runs; at the default 50 000 it
fires once per ~5 000 runs).

Growth expectation (056, debug build, scripted model): a run leaves ~2.3 KB of trajectory and ~2.5 KB
in the main store. At the measured ceiling of ~45 runs/s that is ~390 MB and ~160k files per hour
of trajectory (now capped by the bound above) and ~400 MB per hour in the store (NOT capped).
Spec 116 F2: with trajectories on the sink also held a map entry per run forever (fixed), and its
retention index costs ~250 B of RSS per retained file up to `trajectory_max_files`.

## Memory growth: what slope is expected, what is not (spec 116 F2)

`pii_process_rss_kb` and `pii_runs` give a slope with two polls; `pii_rss_bytes_per_run` does the
division for you: bytes of RSS growth per run between the oldest of up to 32 checkpoints (one per
>= 1 000 runs, taken on `sys.status`/`pii metrics`/`pii health` polls) and now. It is an AVERAGE over
the window, not an instantaneous slope, and it is `0` until 4 000 runs separate the window ends or
while RSS did not grow. The checkpoints live in memory: a daemon restart starts a new window.

Measured (debug build, scripted model, shared host; `scripts/leak_bisect.py`, one persistent
connection, fresh store):

    runs since daemon start    RSS growth per run          why
    0 - ~20 000                ~1.3 KB (1 node) to ~2.5 KB (8 events)   warm-up: caches and allocator fill
    ~20 000 - 32 000+          ~5 - 40 B (plateau)         residual, below; same with every switch below off
    with trajectories ON       +~190-270 B/run until 50 000 files, then flat   the retention index, bounded by trajectory_max_files

- **Read `pii_rss_bytes_per_run` only after the daemon has served ~60 000 runs** (the window then no
  longer contains the warm-up). Before that it is dominated by warm-up and reads high. The warm-up
  is per process, not per store: it happens again after a restart.
- **Expected after warm-up: under ~100 B/run.** A reading that stays above ~100 B/run with the
  daemon past ~60 000 runs is NOT expected: file a bug with `scripts/leak_bisect.py` output.
- RSS also moves with `trajectory_max_files` (the index holds ~190 B per retained file: ~9 MB at the
  default 50 000) and with the number of retained log/audit rotations (not RSS: disk).
- The residual ~5-40 B/run is in the store/runner path, not in the observer fan-out, the audit feed,
  or connection handling (all measured, report 058). It is not explained; at the ceiling of ~100
  runs/s it is a few MB per hour.
- **Fixed in this spec:** with trajectories on, the sink kept one map entry per run it ever saw and
  scanned the whole map on every event (so it also got slower with every run: 150 000 runs through
  the sink alone did not finish in 580 s, and finish in 22 s with the fix). Finished runs are now
  released; a run that hit its per-run cap keeps its (small) entry so a late event stays gated.

## Measurement switches: observer and audit (spec 116 F8)

Both are for MEASUREMENT. Both default to ON; neither is a tuning knob.

| switch | flag | `[core]` key | default | what `off` does |
|---|---|---|---|---|
| observer | `pii daemon --observer on\|off` | `observer = "on"\|"off"` | on | detaches the observer fan-out: runs carry no tool-call observer, the store/provider/net/worker/channel events and out-of-run persist errors are NOT logged (the late-bound log stays unattached and drops them). The bus (client events), the trajectory sink and the audit feed are separate and unaffected. |
| audit | `pii daemon --audit on\|off` | `audit = "on"\|"off"` | on | stops the audit feed of in-run denials (spec 107 R3) and narrows `audit.log` to the single line saying it was switched off. |

The flag wins over `[core]`; anything but `on`/`off` is a usage error (exit 2), never a silent "on".
**Risk:** with `--audit off` a capability, deny-list, scope or egress denial is not recorded anywhere
durable; with `--observer off` a failing store write or a tool call leaves no log line. Do not run
either in production. Switching either off is announced three ways: a stderr line, a `warn` log event
(`daemon.audit.disabled` / `daemon.observer.detached`, both carrying `effect` and
`purpose: measurement only; the default is on`), and the audit trail itself
(`daemon.audit.disabled` is the one record `audit.log` keeps when the trail is off;
`daemon.observer.detached` is an audit kind).

## Write amplification: where a run's bytes go (spec 116 F3)

Measured with the daemon's own `/proc/<pid>/io` (`write_bytes`: bytes handed to the block layer) and
reproduced in-process (`cargo test -p pii-graph --test write_amp -- --nocapture` and
`cargo test -p pii-store --lib write_amp -- --ignored --nocapture`). Per run, 4 000 runs in, default
config (no log file, trajectories off, audit on):

    writer                                  before        after
    run-id counter commit                   ~24 600 B      766 B   (ids reserved 32 at a time)
    graph body commit                       ~24 600 B        0     (content-addressed: not re-committed)
    run input body commit                   ~24 600 B        0     (same)
    journal + header + status (1 commit)    ~50 400 B     50 429 B
      of which fixed cost of ANY commit     24 576 B      (6 x 4 KiB pages: redb's tree + freed-tree + header)
      journal rows (5 events)                8 220 B   (marginal, 1 000-run-deep store)
      run header                             8 929 B   (marginal)
      run_by_status index                    8 219 B   (marginal)
    audit.log                               0 per run (279 B once)
    log file (if configured)                ~205 B (below resolution, spec 056)
    trajectory (if configured)              +4 085 B
    daemon total (leak_bisect.py, lean)   124 150 B      51 236 B   (-58.7%)
    sum of in-process parts (after)                      51 195 B   (0.08% from the daemon's total)

**Tolerance: the parts must sum to the daemon's total within 5%** (they do, within 0.1%: in-process
766 + 50 429 = 51 195 B vs the daemon's 51 236 B). The per-table split is coarser: it is in 4 KiB
pages, and the three cumulative probe commits sum to 49 916 B against 45 274 B for the real
`append_events` (10% apart), so read the split as "each table costs about one tree path, ~8 KB",
not as a precise ledger. The ~50x
of spec 056 (125-131 KB to retain 2.5 KB) was NOT the events: one commit costs ~24.6 KB whatever it
carries, and a run used to make four. It is now one. The remaining 50 KB is that one commit; its
size grows with the store (the btree paths deepen: 120 -> 131 KB per run over 32 000 runs before the
change). Throughput on this host went from ~45 to ~80-120 runs/s because the disk was the ceiling.

**Run ids now have gaps after a restart.** The counter is persisted 32 ahead; a restart skips the
unissued rest of the block (up to 31 ids). Ids are still strictly increasing and never reused.
Durability of a run's journal, header and the audit trail is unchanged: nothing was made
asynchronous and no commit was weakened.

## The store never shrinks on its own (spec 115 R3)

`pii metrics` and `pii health --json` (`notes[]`) say it in one line: **the main store never shrinks
on its own.** Runs and journal events are never deleted, and redb only returns free space at the END
of the file. Watch `pii_store_bytes_per_run` (bytes / runs) and `pii_store_bytes`: the trend of
those two is the whole signal. Plan the disk for ~2.5 KB per run (056: 1.06 MB -> 216.6 MB over
85 018 runs).

`pii store compact [--dir D]` is real but narrow. Stop the daemon first (it refuses while the stores
are held). It runs redb's in-place compaction on each store file and prints the MEASURED size
before and after. It reclaims free pages sitting inside the file (deleted sessions, slack from
preallocation); it cannot shrink data that is still live, and runs/journal events are always live.
Measured:

    fragmented store (20 sessions x 400 KB, every other one deleted):  13 111 296 -> 6 623 232 bytes
    store after 400 runs through a daemon (live data):  store.redb 1 048 576 -> 786 432 (allocation slack)
    idle auxiliary stores (memory/approval/heal/channel/timer):  1 056 768 -> 36 864 each

So on a store that has only grown with runs, expect the tail slack back and nothing more; real
reclamation of run history would need a retention policy for runs, which does not exist.

## MQTT broker (spec 117)

The daemon needs **no broker of its own and ships none**: `[[mqtt]]` points at one you run
(e.g. `mosquitto`, which this host does not have). Set `url = "mqtt://<host>:1883"`, `topics`,
and `graph`; credentials via `password_env`/`password_file` (0600). The client is `rumqttc` 0.25
(`default-features = false`: plain TCP, **no TLS**), so a remote broker with credentials needs
`allow_plaintext_credentials = true` (clear text on the wire) — otherwise use a loopback
broker or a tunnel. Health: `channel.<name>` is `Degraded` with the reason while the broker is
unreachable; retries back off 500 ms → 30 s (`reconnect_*_ms`). `pii metrics` shows
`pii_mqtt_received_total`, `pii_mqtt_dropped_total`, `pii_mqtt_published_total`. See
`_docs/02-guides/mqtt.md`.

## Worker reload: what happens to a request that arrives during one (spec 120)

A reload (`worker.reload`) never silently loses a request. A request is decided in one step:

- **Arrived before the reload began** (already in flight): the reload waits for it, up to 5 s, then the
  old worker is shut down. Anything still unanswered at the deadline is cancelled with
  `ReloadAborted: ...` and the event `worker.request.refused` (`reason=reload_deadline`).
- **Arrived while the worker is draining or restarting**: queued (bounded at 256) and run on the new
  worker once it says hello, in arrival order. A full queue is refused with `reload queue full` and the
  event `worker.request.refused` (`reason=queue_full`); one that waits past its timeout, `queue_timeout`.

Every refusal increments `pii_worker_requests_refused_total` (`pii metrics`). A finished reload emits
`worker.reload.completed` with `in_flight_at_start` (requests waited for), `queued_flushed` (queued
requests run afterwards) and `aborted` (cancelled at the deadline): `0/0/0` is "a reload happened and
nothing was in flight"; a non-zero `aborted` is "the reload cost requests", and each has its own
`worker.request.refused` line. A worker that dies on its own is a different path: its in-flight
requests fail with `WorkerDied` and `worker_died` is recorded; a late exit report from a worker that
a reload already replaced is ignored (it is not a death of the new worker).

## Logs

The daemon writes events as JSON lines (one per event, `ts_ms` stamped) to a
size-rotated file. Off unless a path is set. Keys in `[core]` (flag in brackets
overrides the key):

    log_file         = "/var/log/pii/pii.log"   # [--log-file]
    log_rotate_bytes = 10485760                  # [--log-rotate-bytes]  0 = never rotate
    log_keep_files   = 5                         # [--log-keep-files]    rotated files kept

Rotation: `pii.log` -> `pii.log.1` -> `pii.log.2` ... A log that ends on
`daemon.started` with no `daemon.stopped` means the daemon died hard (kill -9).

## Confinement, audit trail and secrets (spec 107)

**Default: confined.** `pii daemon` applies the `workspace` Landlock profile unless told
otherwise. `--sandbox off` is the explicit opt-out: the daemon then starts, prints
`warning: daemon is running UNCONFINED (--sandbox off)` on stderr, and logs a `warn`
`daemon.sandbox.in_force` event (`enforced=false`). If confinement cannot be enforced
the default start FAILS with exit 4 (it does not fall back to unconfined).
Why this default: confinement exists and is measured working; an unconfined default made
"safe" and "absent" indistinguishable to the operator.

**The exact grant** (one text, quoted by `pii health` mounts[workspace].policy, the
startup event, and this page; pinned by `crates/pii-cli/tests/sandbox_grant.rs`):

- WRITES are allowed only under the workspace, the store dir, the runtime dir, the log
  file's directory, the trajectory directory, `$TMPDIR` and `/dev/{null,zero,urandom,tty}`.
  Everything else is denied (`Permission denied`; an existing file outside is left intact).
- READS are NOT confined: read-only access to `/` is granted, so `/etc`, `/home` and every
  other path the daemon user can read stays readable. That is deliberate (DNS, CA bundles,
  shared libraries, the config file); narrowing reads is not done today.
- The child processes a run spawns inherit the same grant.

**Audit trail.** Denials that pass through pii (bash deny list, bash cwd scope, fscap
`fs_read`/`fs_write`, the http allowlist, any tool returning Denied) are written as
`core.cap.denied` events to `<store dir>/audit.log` (e.g. `/var/lib/pii/audit.log`), on by default
(`--audit off` is a measurement switch, below, never a production setting),
together with `daemon.sandbox.in_force` and `daemon.peer.refused`. Rotation is the log's:
`audit.log` -> `audit.log.1` ... `.5`, rotated at `log_rotate_bytes` (default 10 MiB),
`log_keep_files` kept. Example line from a real run:

    {"fields":{"cap":"tool:bash: command matches deny list ('sudo')","principal":"node:t@run:r000000000001"},"kind":"core.cap.denied","level":"warn","subsystem":"core","ts_ms":1791589942416}

**Limit, stated plainly: a kernel-level denial inside a CHILD process (a raw
`echo > /outside` run by bash) is invisible to pii.** Landlock refuses the write inside the
child; the child sees `Permission denied`, and pii only sees the exit status. Nothing in
`audit.log` records it.

**Secrets.** Decision: secrets are delivered by environment variable only
(`PII_DISCORD_TOKEN`, `OPENAI_API_KEY`, ...) and are never written to the config file
(the config loader is strict and the shipped sample says so). No keyring or secret file is
read by pii today. Compensating controls: (1) keep them in a root-owned mode-0600
`EnvironmentFile=` (see `deploy/pii.service`), which systemd reads before dropping to the
service user; (2) every sink that records run output — the log file, `audit.log`, the
trajectory files, the `pii -p --json` / `pii graph run` ndjson streams — replaces the value of
any environment variable whose name contains `TOKEN`, `SECRET`, `PASSWORD`, `PASSWD`,
`API_KEY`, `APIKEY`, `PRIVATE_KEY`, `CREDENTIAL` or ends in `_KEY` (values of 8+ bytes) with
`[REDACTED]` (test: `crates/pii-cli/tests/canary_secrets.rs`). It matches the exact value: a
secret that is transformed (hashed, split, re-encoded) before it is echoed is not caught.
`PII_TRAJECTORY_REDACT=a,b` additionally drops whole named fields from trajectories.
The client event stream a connected UI receives is not scrubbed.

**seccomp:** not implemented — there is no syscall filter, only Landlock filesystem
confinement, so Landlock's coverage must not be read as syscall-level parity (a cheap real
filter would need an allowlist built from a traced run of the daemon plus its tools; not built).

## Back up and restore

Offline: stop the daemon first. A backup opens each store with redb's exclusive
lock and copies it while holding that lock, so a running daemon makes the
backup REFUSE (`stop the daemon first`) rather than copy a file mid-write. Each
copy is re-opened and its format version compared with the source, and
`MANIFEST.json` records size, version and blake3 per file.

    systemctl stop pii
    pii backup /backups/pii-$(date +%F) --dir /var/lib/pii      # destination must be empty or new
    systemctl start pii

`--dir` is the directory holding the stores (default: the runtime dir). With
the shipped unit the channel/timer stores sit in the runtime dir; back that up
separately with `pii backup <dest2> --dir /run/pii` if you need channel
cursors and timers.

Restore:

    systemctl stop pii
    pii restore /backups/pii-2026-10-09 --dir /var/lib/pii           # refuses if any target store exists
    pii restore /backups/pii-2026-10-09 --dir /var/lib/pii --force   # moves existing stores to <name>.redb.old-<ms>
    systemctl start pii

Restore verifies every file's blake3 against the manifest before touching the
target, never deletes (`--force` moves the old store aside; remove the
`.old-*` files yourself once satisfied), and refuses a store that is in use.

## Upgrade and the version refusal

Every store carries a format version (`store.format_version` in its `meta`
table), stamped on create/first open. This build supports version 1 and
REFUSES to open anything newer:

    error: invalid: store '/var/lib/pii/memory.redb' has format version 99, which is newer than the version 1 this binary understands; upgrade pii or use a different store path

The daemon exits 2 and writes nothing to the file. What to do: install a pii
build that supports that version, or (rollback) restore a backup taken by the
older build (`pii restore ... --force`). Never edit the stamp by hand.
Upgrade procedure: stop, `pii backup`, install the new binary, start, check
`pii health` (every `format_version` as expected).

## Wedged daemon

1. `pii health --json` — no answer within a few seconds = wedged or dead.
2. `systemctl stop pii`. Shutdown waits at most 5 s for in-flight blocking work.
   If something was stuck, stderr (`journalctl -u pii`) carries exactly:
   `pii: shutdown drain exceeded 5s; abandoning in-flight blocking work`
   — the daemon then exits anyway. That line is stderr only, not in the log file.
3. If systemd had to SIGKILL it (no line, no `daemon.stopped` in the log file),
   the store is still a transactional redb file; the daemon reconciles
   interrupted runs on boot. Check `pii health` after restart.
