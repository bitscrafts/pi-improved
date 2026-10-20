# Spec: trace and explain read THROUGH the daemon, never by opening the store

## Overview
Measured 2026-10-20 on the deployed revision:

    pii run trace r000000004481                 -> error: no run r000000004481
    pii run trace r000000004481 --db store.redb -> error: Database already open. Cannot acquire lock

`trace` opens the redb store directly while the daemon owns it, so with the daemon up — the normal state — the command that
is supposed to prove a run's agent calls cannot read them. Spec 217 added coder.spawned/coder.exited; this spec makes them
readable from the outside.

## Requirements
1. `pii run trace <id>` and `pii run explain <id>` ask the DAEMON for the events (RPC), and fall back to a direct store read
   only when no daemon is reachable (and say which path was used).
2. When a run id does not exist, the message names the store that was consulted and the daemon's run count, so "no run" is
   distinguishable from "wrong store".
3. The RPC returns the same events the journal holds: no re-derivation, no summarising on the daemon side.
4. `--db` remains supported for the offline path, and its use is printed.
5. Exit code 0 when events are returned; non-zero with a named reason when the run is absent.

## Exit Criteria
- [ ] `cargo test -p pii-cli --test trace_reads_through_the_daemon`
- [ ] `cargo test -p pii-cli --test trace_absent_run_names_the_store`
- [ ] with the daemon running, `pii run trace <a real run id>` prints its events and does NOT mention a lock
- [ ] the printed path marker says `via=daemon` in the normal case

## Guardrails
- Do NOT retry the direct open in a loop: the lock failure is deterministic, not transient.
- Do NOT make trace mutate anything — read-only, both paths.
- Do NOT hide which source answered: the reader must always be able to tell where a line came from.
