# Live spawn is reaped: the seam between the write and the reaper (spec 292)

## The seam

A host agent's pid reaches the store on the **write** side and leaves it on the **read** side. The write side is
`crates/pii-server/src/node_kinds/nodes/coder.rs::journal_spawned`, which a run's own coder reaches through
`Engine::start_run`: it journals `coder.spawned` with `runtime`, `pid`, `session`, `argv0`, `argv_digest` and the rest, as
the runtime reports the pid and before the coder waits. The read side is `crates/pii-server/src/engine/orphans.rs`:
`reap_orphans` selects a run only when its journal holds more `coder.spawned` than `coder.exited`, and kills the pid of
each unaccounted spawn whose command line still names the program it was spawned as. Boot's recovery
(`engine/session_durability.rs::recover_boot`) calls it, then journals each reap as `coder.orphan_reaped` on its run.

The twelve tests of `crates/pii-server/tests/orphans_are_reaped.rs` could not see a break in the write. Each seeds its
`coder.spawned` with a local `spawned(...)` helper, a hand-built event, so the read side was tested against what the test
wrote, never against what `journal_spawned` writes. A dropped `pid` field, a renamed `argv0`, or a `session` that stopped
being set would leave all twelve green. `crates/pii-server/tests/live_spawn_is_reaped.rs` crosses the seam: it drives a
real run whose coder hangs, reads the spawn out of the store the run wrote, and reaps it from a restarted copy of that
store through the boot path.

## What the new test proves

| Test | Evidence it reads | Asserts |
| --- | --- | --- |
| `a_live_coder_spawn_is_reaped_from_the_journal_the_run_wrote` | the `coder.spawned` the run journalled (`runtime = host`, positive `pid`, non-empty `session` and `argv_digest`) | the journalled pid is the one signalled; it is dead; the run journals `coder.orphan_reaped` for it (`kind = process`, `result = killed`); the run leaves `Running` |
| `a_live_run_that_exited_is_not_searched` | the run's own journal, which holds `coder.exited` for both coders | a finished run is not searched; a copy of it held `Running` with its exits kept is not searched either; nothing is signalled |
| `a_live_run_is_reaped_once_its_exit_is_gone` | the same run's journal copied under a `Running` header, minus the first coder's `coder.exited` | the first spawn's pid is named (`already_gone`: the process has exited) and nothing is signalled, so the control can fail |

The copies are the run's own journalled events, re-appended. The test constructs no `coder.spawned` event and no
`NodeObserved` value (`grep -c NodeObserved` is 0 in the file).

## Measured, not assumed

- **The live run's terminal after the reap.** `reap_orphans` only returns the reaps. The run's own terminal is written by
  boot's `reconcile_with`, which runs in `recover_boot`. The test therefore reaps on a restarted copy of the store, as a
  restarted daemon finds it: the run `Running`, its agent alive. On that copy the reap journals `coder.orphan_reaped`,
  and the run closes `Failed` with the failure text `interrupted by daemon restart; agent reaped`, which the test asserts.
  The original, still-running engine's coder terminal after its agent is killed is **not** asserted: the test does not
  measure it.
- **Whether the harness can hang a coder through `Engine::start_run`.** It can. Overwriting `case.bin/claude` with a
  script that writes `$$` to `bin/pids` and then runs `exec -a "$0" /bin/sleep 100000` makes the coder journal
  `coder.spawned` and then block; `/proc/<pid>/cmdline` starts with the name the coder spawned it as, so the reaper's
  `runs_program` check passes. No event was seeded.
- **The live pid.** The pid differs on each run and the test does not print it; the test asserts it from the journal on
  every run. The pid from the run that produced the digests below is not recorded here.

## The control, step by step

The script is `scripts/checks/live-spawn-reap.sh`. Its mutation is `.field("pid", i64::from(s.pid))` removed from
`journal_spawned`.

| Step | Command | Output |
| --- | --- | --- |
| 1 | `sha256sum crates/pii-server/src/node_kinds/nodes/coder.rs` | `BEFORE a6a57f30b660bcf05e5a22a4318781d009746c182411592e4c9d81359c5595c5` |
| 2 | `cp` the file's bytes into scratch | saved; `cmp` equal |
| 3 | plant the mutation (anchor counted: 1) | `MUTATED removed: .field("pid", i64::from(s.pid))` |
| 4 | `cargo test -q -p pii-server --test live_spawn_is_reaped` | RED: `panicked at crates/pii-server/tests/live_spawn_is_reaped.rs:139:28` (the pid is missing from the `coder.spawned` the run journalled) |
| 5 | `cargo test -q -p pii-server --test orphans_are_reaped` | `test result: ok. 12 passed; 0 failed` (the seeded suite cannot see the break) |
| 6 | restore the saved bytes, `touch` | `RESTORED` |
| 7 | `sha256sum` and `cargo test -q -p pii-server --test live_spawn_is_reaped` | `AFTER a6a57f30b660bcf05e5a22a4318781d009746c182411592e4c9d81359c5595c5`; `test result: ok. 3 passed` |
| 8 | `git status --porcelain crates/pii-server/src` | empty |

`BEFORE` and `AFTER` are equal. The file is byte-identical after the run.

## Verdict

LIVE-SPAWN-REAP: OK
