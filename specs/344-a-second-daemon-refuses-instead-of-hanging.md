# Spec: A second daemon refuses, loudly, instead of hanging

## Overview

Measured 2026-11-02: with one daemon already running, a second `pii daemon` — given its own
`--workspace`, its own `PII_RUNTIME_DIR` and its own `PII_STATE_DIR` — starts, logs
`daemon.starting`, and then does nothing at all. No `daemon.started`, no error on stderr, no
`control.sock`, and no exit: it was still running when a 8 s timeout killed it (`rc=124`).

The cause is the durable store added by row 337. Its redb files are exclusive, so the second
process blocks on the lock. Blocking is the correct decision — one daemon per user, one set of
stores — but **blocking silently is not**: an operator who starts a second daemon sees a process
that is up and a `pii health` that says "no daemon answered", with nothing anywhere naming the
conflict. In the test suite it cost an afternoon: five test files spawn daemons in parallel, each
second daemon hung, and every failure surfaced as
`connect …/control.sock: No such file or directory` — which reads like a broken client.

**Project**: <repo>
**Phase**: path to production — the daemon must be operable by someone who is not its author.

## Requirements

### Functional

1. When the durable store cannot be locked because another process holds it, the daemon MUST exit
   within 10 seconds with a message naming (a) the store path it could not lock and (b) that
   another daemon holds it.
2. The failure MUST be `cli.failure` with `stage: "daemon"` and reason `store_locked`, so it is
   greppable in the log the way `startup_failed` already is.
3. Its exit code MUST be distinguishable from a successful run and from a bad config: `3`.
4. A daemon that IS the only one MUST still start exactly as today (no new required config, no new
   flag, no measurable startup cost beyond one non-blocking probe).
5. The lock probe MUST NOT delete, truncate or migrate anything: a refused start leaves the running
   daemon's stores untouched.

### Non-Functional

- The refusal is a *probe* before the store open, not a timeout around it: no `sleep`, no retry loop.
- One attempt, one message, one exit. Nothing in the daemon retries.
- **The test harness must take a slot before it spawns** (`daemon_guard::spawn`, one per test
  binary, released when the guard drops). Until it does, every binary whose tests each start a
  daemon fails in parallel and passes with `--test-threads=1` — measured on
  `project_config_layers_live` (6 passed serially in 0.60 s, 4 failed in parallel) and on
  `control_plane_is_not_behind_runs` (6 passed in 32 s behind a lock, 5 failed without one).
  The slot must be re-entrant per process, or a test that starts two daemons waits on itself.

## Architecture

```
cmd_daemon::run
  └─ durable::claim(dir)            (new: non-blocking)
       ├─ Ok(handle)       -> bind socket, log daemon.started  (unchanged path)
       └─ Err(Locked{path}) -> log cli.failure{reason: store_locked, store: path}
                               exit 3                          (never reaches bind)
```

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| `a_second_daemon_refuses_and_says_which_store` | daemon A running; start B with its own dirs | B exits 3 within 10 s; stderr names the store path |
| `the_refusal_names_the_running_daemon` | same | the message contains `another daemon` |
| `CONTROL_one_daemon_still_starts` | no daemon running; start A | A logs `daemon.started` and binds `control.sock` |
| `CONTROL_a_refused_start_disturbs_nothing` | A running; B refused | A's socket still answers; A's store mtime unchanged |

## Exit Criteria

- [ ] `cargo test -q -p pii-cli --test second_daemon_refuses`
- [ ] `cargo test -q -p pii-store --lib durable::claim`
- [ ] `bash scripts/checks/gates.sh`

## Guardrails

- Do NOT make the durable stores non-exclusive to dodge this: exclusivity is what keeps two daemons
  from corrupting one store.
- Do NOT turn the refusal into a retry-with-timeout; a daemon that waits is the defect.
- Do NOT change the exit code of any other failure path.
- Do NOT touch the running daemon's files while probing.

## Files

- `crates/pii-store/src/durable.rs`
- `crates/pii-cli/src/cmd_daemon.rs`
- `crates/pii-cli/tests/second_daemon_refuses.rs`
