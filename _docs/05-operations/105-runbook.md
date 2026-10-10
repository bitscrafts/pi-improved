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
    pii_store_bytes_per_run            gauge    main-store bytes / runs: the growth rate as one number
    pii_store_bytes{store="main"}      gauge    file size, one line per store
    pii_store_format_version{store=…}  gauge    persisted format version, one line per store
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
`core.cap.denied` events to `<store dir>/audit.log` (e.g. `/var/lib/pii/audit.log`), always on,
together with `daemon.sandbox.in_force` and `daemon.peer.refused`. Rotation is the log's:
`audit.log` -> `audit.log.1` ... `.5`, rotated at `log_rotate_bytes` (default 10 MiB),
`log_keep_files` kept. Example line from a real run:

    {"fields":{"cap":"tool:bash: command matches deny list ('sudo')","principal":"node:t@run:r000000000001"},"kind":"core.cap.denied","level":"warn","subsystem":"core","ts_ms":1791589942416}

**Limit, stated plainly: a kernel-level denial inside a CHILD process (a raw
`echo > /outside` run by bash) is invisible to pii.** Landlock refuses the write inside the
child; the child sees `Permission denied`, and pii only sees the exit status. Nothing in
`audit.log` records it.

**Secrets.** Decision: secrets are delivered by environment variable only
(`PII_DISCORD_TOKEN`, `OLLAMA_API_KEY`, ...) and are never written to the config file
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
