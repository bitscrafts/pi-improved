# Spec 292: the reaper is proven on a spawn a real run journalled, not one a test built by hand

## Overview
**Project**: /projects/pi-improved
**Phase**: production — the safety net that kills a dead daemon's agents. Spec 242 built `reap_orphans` and certified
it on **hand-built events**: every one of the twelve tests in `crates/pii-server/tests/orphans_are_reaped.rs` seeds its
`coder.spawned` with the local helper `spawned(...)` (a `RunEvent::NodeObserved` literal, `session: ""`), so no test
connects the **write** side of the seam (`coder.rs::journal_spawned`, the live path) to the **read** side
(`engine/orphans.rs`, which selects a run by `coder.spawned` minus `coder.exited` and kills the pid it finds). A defect
on either side of that seam passes all twelve. This spec adds the test that crosses it, and the control that proves the
seam was uncovered.

This is spec 289's recorded Follow-up, and the class the loop has paid for repeatedly (`DO-NOT-REPEAT 1.50`: a value a
test asserts but the live path never reads is a false green).

## Verified Premises
- **CONFIRMED** (read 2026-10-27) `crates/pii-server/tests/orphans_are_reaped.rs` — 527 lines, 12 `#[test]` functions,
  every spawn seeded through `fn spawned(runtime, pid, argv0, container)` at line 167; `a_real_orphan_process_is_killed`
  (line 506) starts a real `sleep` and seeds the event that names it, so the **process** is real but the **journal** is
  not.
- **CONFIRMED** `crates/pii-server/src/engine/orphans.rs:114` selects a run only when `coder.spawned` outnumbers
  `coder.exited` in `store.read_journal(&run)`, and `reap_processes` kills only a spawn whose `runtime` is `host` and
  whose `pid` is a positive integer absent from the exits.
- **CONFIRMED** the live writer is `crates/pii-server/src/node_kinds/nodes/coder.rs::journal_spawned` (line ~343), which
  fields `agent`, `model`, `runtime`, `image`, `container`, `argv0`, `argv_len`, `argv_digest`, `pid`, `session`,
  `skill`, `sha256`; the pid it names is handed to the runtime's `spawned` callback inside `journalled_run` (line ~301).
- **CONFIRMED** a run-level harness already exists and is shared by two test targets:
  `crates/pii-server/tests/spec_graph_daemon/mod.rs` (470 lines) — `in_child(test, docs)` re-executes the test binary with
  a fake-agent bin directory first on `PATH` (the workspace forbids `set_var`), `Case::new(bin, criterion)` builds a
  throwaway repo, `Case::engine(graphs)` / `engine_on_store` build the `Engine`, `observed(journal, kind)` reads a kind
  out of a journal, `alive(pid)` asks with signal 0, and `Sleepers` kills a group on `Drop`. Its `DOCS_HANG` body already
  proves the shape a hang needs: `echo $$ >> …; exec -a "$0" /bin/sleep 100000`, so `/proc/<pid>/cmdline` starts with
  the path the agent was spawned as.
- **MEASURE FIRST (1)** — before writing an assertion, read `reap_orphans` and say in the spec's Closure **what the run's
  own terminal is** after a live coder's agent is killed by the reaper (does the coder node's wait return an error that
  fails the run, and does the run's failure text carry the reap sentence `interrupted by daemon restart; agent reaped`?).
  Assert what the code does; do not assume the seeded tests' `the_failure_text_says_the_agent_was_reaped` shape carries
  over to a live run.
- **MEASURE FIRST (2)** — whether a hanging coder can be produced by overwriting the harness's `claude` in `case.bin`
  (the `bin` field is `pub`) with a hang-on-any-brief script. Preferred: it touches no shared file. If the harness cannot
  produce a coder that reaches `coder.spawned` and then hangs through `Engine::start_run`, **STOP and report** — do not
  fall back to seeding the event, which is the defect this spec removes.
- **NOT CLAIMED** that the twelve seeded tests are worthless: they pin the reaper's selection and kill rules well. The
  claim is narrower and measurable — none of them can see a break in the live write, which R2 proves.

## Requirements
### Functional
- **R1. The test that crosses the seam** — new file `crates/pii-server/tests/live_spawn_is_reaped.rs`, target
  `live_spawn_is_reaped`, using `mod spec_graph_daemon;` (as `spec_graph_runs_through_the_daemon.rs` does):
  - (a) **Live evidence.** Drive one run through `Engine::start_run` on a graph whose coder's agent hangs, and read the
    spawn out of the store the run wrote: `store.read_journal(&run)` holds a `RunEvent::NodeObserved` with kind
    `coder.spawned` whose `runtime` is `host`, `pid` is a positive integer, and `session`/`argv_digest` are non-empty
    (the fields only the live writer produces). Every wait is bounded by a deadline (poll the journal, at most 10 s,
    50 ms apart) and a missed deadline **panics printing the journal's kinds** — the test fails, it never hangs.
  - (b) **The reap.** Call `reap_orphans(&engine.store, &ctx, &runtime, &ProcFs)` — the same function the boot path
    calls. Assert, with a deadline: the pid **the run journalled** is dead (`alive(pid)` false), the run's journal gains
    `coder.orphan_reaped` naming that pid, and the run has left `Running` (its terminal asserted per MEASURE FIRST (1)).
  - (c) **Control, on the live path.** In the same harness, a second run whose coder **exits normally** journals
    `coder.exited`, and the reaper's return value names nothing for it and signals no process — a run that finished is
    not searched. This control must be able to fail: the reaper is also called once with the exit event removed, and
    then the same run **is** reaped (the near-miss half).
  - (d) **No hand-built event.** The file contains no `NodeObserved` construction and no local `spawned(...)` helper: the
    only `coder.spawned` it can see is the one the run wrote. It must not use `orphans_are_reaped.rs`'s `seed`.
  - (e) **Nothing outlives the test.** Every process it starts is killed on the way out (a `Drop` guard; `Sleepers` from
    the harness may be used), including when an assertion fails early.
- **R2. The control that shows the seam was uncovered** — new script `scripts/checks/live-spawn-reap.sh` (the
  `scripts/checks/` family: `set -u`, own scratch under `/projects/_tmp`, prints one verdict line, exit 0 only on
  success). Its steps, in this order:
  1. `sha256sum crates/pii-server/src/node_kinds/nodes/coder.rs` → `BEFORE`;
  2. copy the file's **bytes** into its own scratch directory (never a hardlink, never a move);
  3. plant the mutation with python: remove the single line `.field("pid", i64::from(s.pid))` from `journal_spawned`.
     Refuse (`exit 3`, `NO-APPLY`) unless the anchor occurs exactly once, writing nothing;
  4. `cargo test -q -p pii-server --test live_spawn_is_reaped` **must fail**, and the output must name a failing
     assertion from that file (print the first `panicked at` line);
  5. `cargo test -q -p pii-server --test orphans_are_reaped` **must pass, all 12** — this line is the measurement: the
     seeded suite cannot see a break in the live write. If any of the twelve turns red, the claim of R2 is wrong: report
     it as `SEAM-ALREADY-COVERED` and exit 1 rather than rewriting the argument;
  6. restore: write the saved **bytes** back and `touch` the file. **Never `cp -p`** — it keeps the original mtime and
     the next `cargo test` reuses the mutated binary (the false-red trap measured 2026-10-18 on spec 187);
  7. `sha256sum` equals `BEFORE`, then `cargo test -q -p pii-server --test live_spawn_is_reaped` passes again;
  8. last line `LIVE-SPAWN-REAP: OK`, exit 0. Any deviation prints `LIVE-SPAWN-REAP: FAIL <why>` and exits 1; a mutation
     that does not compile prints `LIVE-SPAWN-REAP: FAIL does not compile` and is not counted as a red.
  The script leaves the tree byte-identical: `git status --porcelain crates/pii-server/src` must be empty afterwards.
- **R3. The record** — new `_docs/05-operations/LIVE-SPAWN-REAP.md`: the seam in one paragraph (write path, read path,
  why twelve seeded tests could not see it), a table row per step of R2 with its output, the `BEFORE`/`AFTER` digests,
  the live pid the run journalled, and the sentence MEASURE FIRST (1) settles. Ends with the literal line
  `LIVE-SPAWN-REAP: OK`.

### Non-Functional
- One new test target, one new check script, one new document. **No change under `crates/*/src`** (R2's mutation is
  planted and restored inside one run, byte-identical) and no change to `crates/pii-server/tests/spec_graph_daemon/mod.rs`
  or `orphans_are_reaped.rs`. No new dependency, no `unsafe`, every file under the 450-effective-line gate.

## TDD Contract

| Test Name | Input | Expected Output |
| --- | --- | --- |
| `a_live_coder_spawn_is_reaped_from_the_journal_the_run_wrote` | a run through `Engine::start_run` whose coder's agent hangs; the journal read back from the store; `reap_orphans` | the journalled pid is dead, the journal holds `coder.orphan_reaped` for it, the run has left `Running` |
| `a_live_run_that_exited_is_not_searched` | the same harness, a coder that exits normally (journal holds `coder.exited`) | the reaper returns nothing for that run and signals no pid |
| `a_live_run_is_reaped_once_its_exit_is_gone` | the same run's `coder.exited` removed from a copy of its events (the near-miss) | the same run **is** reaped — the two cases differ only by the exit, so the control can fail |
| `scripts/checks/live-spawn-reap.sh` | the unmutated tree | `LIVE-SPAWN-REAP: OK`, exit 0, `git status` on `crates/pii-server/src` empty |
| `scripts/checks/live-spawn-reap.sh` (under its own mutation) | `journal_spawned` without the `pid` field | the new test RED, the twelve seeded tests GREEN, restore digest equal |

## Exit Criteria
- [ ] `cargo test -q -p pii-server --test live_spawn_is_reaped 2>&1 | grep -qE "^test result: ok\. [1-9][0-9]* passed"`
- [ ] `test "$(grep -c '^#\[test\]' crates/pii-server/tests/live_spawn_is_reaped.rs)" -ge 3`
- [ ] `! grep -q 'NodeObserved' crates/pii-server/tests/live_spawn_is_reaped.rs && grep -q 'start_run' crates/pii-server/tests/live_spawn_is_reaped.rs`
- [ ] `cargo test -q -p pii-server --test orphans_are_reaped 2>&1 | grep -qE "^test result: ok\. 12 passed"`
- [ ] `bash scripts/checks/live-spawn-reap.sh 2>&1 | tail -1 | grep -q "LIVE-SPAWN-REAP: OK"`
- [ ] `grep -q 'LIVE-SPAWN-REAP: OK' _docs/05-operations/LIVE-SPAWN-REAP.md`
- [ ] `git diff --quiet -- crates/pii-server/src crates/pii-server/tests/orphans_are_reaped.rs crates/pii-server/tests/spec_graph_daemon/mod.rs`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

## Files
- `crates/pii-server/tests/live_spawn_is_reaped.rs`
- `scripts/checks/live-spawn-reap.sh`
- `_docs/05-operations/LIVE-SPAWN-REAP.md`

## Guardrails
- **Do NOT seed the event.** No `RunEvent::NodeObserved { … coder.spawned … }` construction, no copy of `spawned(...)`,
  no `orphans_are_reaped.rs` helper reuse. The evidence must be the journal a run wrote. If the harness cannot produce a
  hanging coder through `Engine::start_run`, **STOP and report** the measurement — a seeded fallback is the defect.
- **Do NOT weaken, delete or `#[ignore]` an existing assertion**, and do not edit `orphans_are_reaped.rs`,
  `spec_graph_daemon/mod.rs` or anything under `crates/*/src`.
- **Do NOT `cp -p` a restore.** Write the bytes back, `touch`, and verify `sha256sum` before re-running: a stale-binary
  failure is not evidence that the mutation was reproduced.
- **Every wait has a deadline.** A poll loop that can spin forever, or a `sleep` with no bound, is a gate that hangs
  instead of a test that fails.
- **Do not commit.** Do not touch `specs/` beyond reading this file.
- **450 effective lines per file.** If the new test needs more than the gate allows, split it into a directory module the
  way `spec_graph_daemon/` is split, and say so.

## Closure
(pending — the orchestrator records the criteria runs, the mutation's red assertion, the restored digests and the run's
pid here after the item lands)

## Follow-ups
Once the seam is crossed here, the same reading is owed to the other hand-built certifications the reaper's neighbours
still rely on: `crates/pii-server/tests/orphan_is_countable.rs` and the `pii run ls` orphan counter read events they
build, not events a run wrote.
