# Live orphan count: the counter is read from a run a daemon wrote (spec 297)

## The seam

The **writer** is `crates/pii-server/src/node_kinds/nodes/coder.rs::journal_spawned`, reached by a run's own coder through
`Engine::start_run`: it journals `coder.spawned` with the `pid` the runtime reported, among its other fields. The **reader**
is `crates/pii-graph/src/agent_calls.rs::store_totals`, which walks every run header, counts `coder.spawned` events, and buckets
the spawns with no `coder.exited` (`unmatched_spawns`, keyed on the event's `pid`) by the run's header status: a terminal run's
open spawns are `orphans_total` (and `orphans_since_start` when the header's `process_start_ms` is this process's start), a
running run's are `agents_in_flight`. The daemon copies those three numbers into `daemon/identity.rs` (lines 294-296), which is
the health surface. The seeded test, `orphan_is_countable.rs`, cannot see a break in the writer: it builds its
`coder.spawned` with `pid: 4242` and its own `RunHeader` with a constant origin, then passes that constant back, so a spawn
journalled without its pid, or a header stamped with the wrong origin, leaves it green. The new test,
`live_orphan_is_countable.rs`, drives a real run whose agent hangs, takes the pid from the journal that run wrote, reads the
origin from the run's header, and reads the counter over that store.

## Steps of the check (`scripts/checks/live-orphan-count.sh`)

Run of 2026-10-27, unmutated tree first; each mutation is restored byte-identically before the next step.

| Step | What it does | Output |
|---|---|---|
| 1 | Digest of `coder.rs`, bytes saved to scratch | `BEFORE_W a6a57f30b660bcf05e5a22a4318781d009746c182411592e4c9d81359c5595c5` |
| 2 | m1 (write side): remove `.field("pid", i64::from(s.pid))`, anchor counted once | `MUTATED m1 removed: .field("pid", i64::from(s.pid))` |
| 3 | `live_orphan_is_countable` must be red | `RED` at `live_orphan_is_countable.rs:274` (run A's `Succeeded` assert: the run without a pid fails); the hanging-run tests stop earlier, at the `a coder.spawned with a pid never reached` gate |
| 4 | `orphan_is_countable` must stay green (seam not already covered) | `SEEDED test result: ok. 1 passed; 0 failed` |
| 5 | `live_spawn_is_reaped` is expected red (spec 292's seam) | `EXPECTED-RED live_spawn_is_reaped` (`panicked at spec_graph_daemon/mod.rs:94`) |
| 6 | Restore `coder.rs` from scratch, `touch`, digest compared | `AFTER_W a6a57f30b660bcf05e5a22a4318781d009746c182411592e4c9d81359c5595c5` (equal) |
| 7 | m2 (read side): `totals.orphans_total += open;` → `totals.orphans_total += 0;`, anchor counted once | `MUTATED m2 totals.orphans_total += open; -> totals.orphans_total += 0;` |
| 8 | `live_orphan_is_countable` must be red under m2 | `RED` at `live_orphan_is_countable.rs:241` (the terminal run's `orphans_total == 1` assert) |
| 9 | Restore `agent_calls.rs`, `touch`, digest compared; same target green again | `BEFORE_R bba48af44b00decd1dbf159f1d2c2051b367c2fa3fa7e1047d0110aa80bc6a4b`, `AFTER_R` the same, `GREEN test result: ok. 3 passed` |
| 10 | Source directories clean | `git status --porcelain crates/pii-server/src crates/pii-graph/src` empty |

Digests of both files, unmutated and restored:

- `crates/pii-server/src/node_kinds/nodes/coder.rs` — before `a6a57f30b660bcf05e5a22a4318781d009746c182411592e4c9d81359c5595c5`, after `a6a57f30b660bcf05e5a22a4318781d009746c182411592e4c9d81359c5595c5`
- `crates/pii-graph/src/agent_calls.rs` — before `bba48af44b00decd1dbf159f1d2c2051b367c2fa3fa7e1047d0110aa80bc6a4b`, after `bba48af44b00decd1dbf159f1d2c2051b367c2fa3fa7e1047d0110aa80bc6a4b`

Under m1 the first two live tests go red at the gate that waits for a `coder.spawned` with a positive pid, which is the write
side's defect seen before the counter is ever read; the third goes red because the run whose spawn has no pid fails. The
counter's own arithmetic is pinned by the seeded test, which stays green; the claim is that it cannot see the write.

## The live run, measured (not assumed)

Measured with the test body run in-process (`PII_SPEC_GRAPH_DAEMON_BIN` set), at the point after `recover_boot` and `settle`:

- **Live pid journalled by the run:** `693718` (the hanging coder's `coder.spawned`, `runtime = host`, with a session and an argv digest).
- **Terminal state after the terminal `recover_boot`:** `Failed`, the last event `Finished { terminal: Failure { error: "interrupted by daemon restart; agent reaped" } }`.
- **Reap journalled:** `coder.orphan_reaped` on `code`, `kind = process`, `id = 693718`, `result = killed`, `run = r000000000001`.
- **Header origin (sentence (c)):** the live run's `process_start_ms` is `1793141063267`, and the restarted store's header for the same run carries the same `1793141063267`. So the origin the test reads from the header is the one the live path stamped, it survives the restart, and `orphans_since_start == 1` holds. The test reads the origin from the restarted header, and the two agree.

## Test shape, and one deviation from the spec's numbers

- The three tests (`a_live_spawn_is_in_flight_while_its_run_runs`, `a_live_spawn_is_an_orphan_once_the_run_is_terminal`,
  `a_live_run_that_exited_is_not_counted`) run through `Engine::start_run` with `case.input(true)`. The fake agent is replaced
  with the hanging body, and `Sleepers` kills every recorded pid on the way out (Drop), including on a failed assertion.
- The spec's TDD table says `dispatches_total == 2` over the two control runs. **That number is not what the store holds.** A
  spec-graph run with a docs brief journals two `coder.spawned` events (its docs coder and its code coder), and the reaped run
  journalled only one before its agent hung. The measured total is `dispatches_total == 3`. The test therefore asserts the
  dispatch total equals the `coder.spawned` events the two runs' own journals hold, which is the property the spec is after
  (every journalled spawn is a dispatch); the orphan count is still asserted to be exactly `1`, not `2` and not `0`.
- The restart uses `restarted_when_durable`, which retries until the copied store holds the spawn, because a spawn is durable
  within two seconds of the agent starting (spec 289), not at that instant.

LIVE-ORPHAN-COUNT: OK
