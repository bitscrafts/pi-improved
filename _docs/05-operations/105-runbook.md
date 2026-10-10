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
    pii_store_bytes{store="main"}      gauge    file size, one line per store
    pii_store_format_version{store=…}  gauge    persisted format version, one line per store

The contract exposes no latency histogram through `sys.status`, so none is emitted.

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
