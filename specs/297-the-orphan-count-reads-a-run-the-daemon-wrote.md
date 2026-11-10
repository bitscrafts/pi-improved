# Spec 297: the orphan count is read from a run a daemon wrote

## Overview
**Project**: <repo>
**Phase**: production — the counter the health surface reports. `crates/pii-server/tests/orphan_is_countable.rs`
(spec 217 R9) certifies `pii_graph::agent_calls::store_totals` on **hand-built events**: its one test constructs the
`coder.spawned` with the local helper `observed(...)` naming `pid: 4242` (a process that never existed) and its own
`RunHeader` with `process_start_ms = 1_000`, then passes `1_000` back to the counter. Nothing connects the **write** side
(`node_kinds/nodes/coder.rs::journal_spawned`, the live path, the same seam spec 292 crossed for the reaper) to the
**read** side (`store_totals`, whose numbers reach `daemon/identity.rs` at lines 294-296 and so the health surface). A
defect on that seam — a spawn journalled without its `pid`, a header whose origin is not the run's — leaves the seeded
test green and the reported orphan count wrong. This spec adds the test that crosses the seam.

This is spec 292's recorded Follow-up, and the class the loop has paid for repeatedly (`DO-NOT-REPEAT 1.50`: a value a
test asserts but the live path never reads is a false green).

## Requirements
### Functional
- **R1. The test that crosses the seam** — new file `crates/pii-server/tests/live_orphan_is_countable.rs`, target
  `live_orphan_is_countable`, using `mod spec_graph_daemon;` (as `live_spawn_is_reaped.rs` does):
  - (a) **A live run, and the counter's own rule while it runs.** Drive one run through `Engine::start_run` on a graph
    whose coder's agent hangs (`Spec 292`'s shape: overwrite the fake `claude` in `case.bin` with the
    `exec -a "$0" /bin/sleep 100000` body, so `/proc/<pid>/cmdline` names the agent and `Sleepers` can kill the group).
    Wait, bounded by a deadline, until the store's journal holds a `coder.spawned` with a positive `pid` — a pid the
    run wrote, never one the test chose. Then assert `store_totals(&store, origin)` reports `orphans_total == 0` and
    `agents_in_flight >= 1`: a running run's spawn is in flight, not an orphan.
  - (b) **The same run once it is terminal.** Close the run the way a restarted daemon does — `restarted(&case, …)` plus
    `recover_boot(&ctx(), &NoContainers, &Recording::default())`, as spec 292 measured — and assert, with a deadline,
    `store_totals` now reports `orphans_total == 1` for that run's spawn and `agents_in_flight` has dropped by the same
    amount it rose in (a). The counter's arithmetic is unchanged; what changes is that the evidence is a journal a run
    wrote.
  - (c) **`origin` comes from the run, not from the test.** The `process_start_ms` the test passes to `store_totals` is
    read out of the run's own header (`store.get_run(&run).process_start_ms`), so `orphans_since_start == 1` also holds.
    A literal constant would re-make the defect: the header's origin is the value the live path stamps, and the counter's
    `header.process_start_ms == process_start_ms` test (`agent_calls.rs:265`) is only exercised if the test uses it.
  - (d) **A two-sided control that can fail.** A second live run whose coder **exits normally** journals `coder.exited`;
    over both runs `store_totals` must report `dispatches_total == 2` and `orphans_total == 1` — not 2 (a counter that
    treats every spawn as an orphan turns this red) and not 0 (a counter that counts nothing turns the assertion in (b)
    red). The two runs differ only in whether the agent exited, so the control fails when either rule is broken.
  - (e) **No hand-built event.** The file contains no `RunEvent::NodeObserved` construction and no local `spawned(...)`
    helper: reading is done through the harness's `observed(journal, kind)`. It must not reuse
    `orphan_is_countable.rs`'s `observed(...)` constructor.
  - (f) **Nothing outlives the test.** Every process it starts is killed on the way out (`Sleepers`, and a `Drop` guard),
    including when an assertion fails early.
- **R2. The control that shows the seam was uncovered** — new script `scripts/checks/live-orphan-count.sh` (the
  `scripts/checks/` family: `set -u`, own scratch under `_tmp/` INSIDE the repository (`$root/_tmp`, gitignored; amended 2026-10-27: the judge runs criteria through the daemon under landlock `workspace`, which refuses writes under `<scratch>` and allows the repository's own `_tmp`; the first two runs of this spec were refused at verify 7/8 on exactly that). `LIVE_ORPHAN_SCRATCH` may still override it, one verdict line, exit 0 only on success).
  It plants **two** mutations in sequence, each restored byte-identically:
  1. `sha256sum crates/pii-server/src/node_kinds/nodes/coder.rs` → `BEFORE_W`; copy its **bytes** into scratch (never a
     hardlink, never a move).
  2. **m1, the write side.** Plant with python: remove the single line `.field("pid", i64::from(s.pid))` from
     `journal_spawned`. Refuse (`exit 3`, `NO-APPLY`) unless the anchor occurs exactly once, writing nothing.
  3. `cargo test -q -p pii-server --test live_orphan_is_countable` **must fail**, naming a failing assertion from that
     file (print the first `panicked at` line) — and `cargo test -q -p pii-server --test orphan_is_countable` **must
     pass, 1 test**: that green line is the measurement, the hand-built certification cannot see a break in the live
     write. If the seeded test turns red, the claim of R2 is wrong: print `SEAM-ALREADY-COVERED` and exit 1 rather than
     rewriting the argument. `live_spawn_is_reaped` is also expected red under m1 (it is spec 292's seam); print it as
     `EXPECTED-RED` so no reader mistakes it for a surprise.
  4. Restore the saved bytes, `touch` the file, and require `sha256sum` equals `BEFORE_W`. **Never `cp -p`** — it keeps
     the original mtime and the next `cargo test` reuses the mutated binary (the false-red trap measured 2026-10-18 on
     spec 187).
  5. **m2, the read side.** `sha256sum crates/pii-graph/src/agent_calls.rs` → `BEFORE_R`, copy the bytes, and change
     `totals.orphans_total += open;` to `totals.orphans_total += 0;` (refuse unless the anchor occurs exactly once).
     `cargo test -q -p pii-server --test live_orphan_is_countable` **must fail**: the new test is not vacuous. Restore,
     `touch`, digest equals `BEFORE_R`, and the same target passes again.
  6. Last line `LIVE-ORPHAN-COUNT: OK`, exit 0. Any deviation prints `LIVE-ORPHAN-COUNT: FAIL <why>` and exits 1; a
     mutation that does not compile prints `LIVE-ORPHAN-COUNT: FAIL does not compile` and is not counted as a red.
  The script leaves the tree byte-identical: `git status --porcelain crates/pii-server/src crates/pii-graph/src` empty.
- **R3. The record** — new `_docs/05-operations/LIVE-ORPHAN-COUNT.md`: the seam in one paragraph (writer, reader, the
  health surface it reaches, why the seeded test could not see it), a table row per step of R2 with its output, the
  `BEFORE`/`AFTER` digests of both files, the live pid the run journalled and the terminal `recover_boot` left the run
  in (measured, not assumed), and the sentence (c) settles about the header's origin. Ends with the literal line
  `LIVE-ORPHAN-COUNT: OK`.

### Non-Functional
- One new test target, one new check script, one new document. **No change under `crates/*/src`** (R2's mutations are
  planted and restored inside one run, byte-identical) and no change to `crates/pii-server/tests/spec_graph_daemon/mod.rs`,
  `orphan_is_countable.rs` or `live_spawn_is_reaped.rs`. No new dependency, no `unsafe`, every file under the
  450-effective-line gate.

## Verified Premises
- **CONFIRMED** (read 2026-10-27) `crates/pii-graph/src/agent_calls.rs:237` `store_totals(store, process_start_ms)` walks
  every run header, counts per-kind events, then buckets `unmatched_spawns(journal).len()` by header status
  (`:262-270`): `Succeeded | Failed | Refused` → `orphans_total` (and `orphans_since_start` only when
  `header.process_start_ms == process_start_ms`); `Running | Suspended` → `agents_in_flight`.
- **CONFIRMED** `unmatched_spawns` (`:69-89`) keys on the event's `pid` field and `continue`s when a spawn carries no
  numeric pid (`:75-77`), so a spawn journalled without its pid is invisible to the counter — the m1 defect.
- **CONFIRMED** `crates/pii-server/tests/orphan_is_countable.rs` is 82 lines with one `#[test]`, constructs its spawn as
  `observed("coder.spawned", json!({"pid": 4242, …}))` (`:39`), writes its own `RunHeader` with
  `process_start_ms: PROCESS_START_MS` (`1_000`, `:10`), and passes that same constant to `store_totals` (`:45`, `:68`).
- **CONFIRMED** the live writer is `crates/pii-server/src/node_kinds/nodes/coder.rs::journal_spawned`, which fields
  `pid` among `agent`, `model`, `runtime`, `image`, `container`, `argv0`, `argv_len`, `argv_digest`, `session`, `skill`,
  `sha256` — the seam spec 292 crossed for the reaper (`specs/292-…`, committed `44ba72a`).
- **CONFIRMED** `crates/pii-server/tests/live_spawn_is_reaped.rs` (343 lines) already produces a live **hanging** coder
  through `Engine::start_run`: `hang(bin)` (`:98`) rewrites `claude` to `exec -a "$0" /bin/sleep 100000`, `wait_for`
  (`:116`) polls at most 200 × 50 ms and panics with the journal on a miss, `restarted(&case, …)` plus
  `recover_boot(&ctx(), &NoContainers, &Recording::default())` closes the run and its reason carries
  `interrupted by daemon restart; agent reaped`. The harness is `crates/pii-server/tests/spec_graph_daemon/mod.rs`
  (`in_child`, `Case::new/engine/engine_on_store`, `observed`, `Sleepers`, `DOCS_HANG`).
- **CONFIRMED** the daemon's reader is `crates/pii-server/src/daemon/identity.rs:294-296`, which copies
  `orphans_total`, `orphans_since_start` and `agents_in_flight` straight out of `store_totals` — the numbers the health
  surface reports.
- **MEASURE FIRST (1)** — the terminal `recover_boot` leaves a reaped run in (Succeeded, Failed or Refused) is not
  assumed from spec 292's prose: the test reads the header after `settle` and the spec's Closure records which one, with
  the journal's reason. Every branch of `store_totals` that a terminal run can take counts the orphan, so no branch may
  be quietly skipped by the assertion.
- **MEASURE FIRST (2)** — whether the restarted store's header keeps the **original** `process_start_ms` (so
  `orphans_since_start` is 1). Requirement (c) is written to take the value from the header either way, and the Closure
  records what it was; if the restarted header carries a different origin, the test says so on a `NOTE` line rather than
  asserting a number it cannot know.
- **NOT CLAIMED** that `orphan_is_countable.rs` is worthless: it pins the counter's own arithmetic (the in-flight versus
  terminal split, the exactly-once increment) well. The claim is narrower and measurable — it cannot see a break in the
  live write, which R2 step 3 buys a green line to prove.

## TDD Contract
| Test | Input | Expected |
|---|---|---|
| `a_live_spawn_is_in_flight_while_its_run_runs` | a run through `Engine::start_run` whose coder's agent hangs; the store's journal | `orphans_total == 0`, `agents_in_flight >= 1` |
| `a_live_spawn_is_an_orphan_once_the_run_is_terminal` | the same run after `recover_boot` closes it; `origin` read from its header | `orphans_total == 1`, `orphans_since_start == 1`, in-flight drops by what it rose |
| `a_live_run_that_exited_is_not_counted` | two live runs, one reaped and one exited | `dispatches_total == 2`, `orphans_total == 1` (not 2, not 0) |
| `scripts/checks/live-orphan-count.sh` | the unmutated tree | `LIVE-ORPHAN-COUNT: OK`, exit 0, `git status` on `crates/*/src` empty |
| `scripts/checks/live-orphan-count.sh` (m1, write side) | `journal_spawned` without the `pid` field | new test RED, `orphan_is_countable` GREEN — the hole measured |
| `scripts/checks/live-orphan-count.sh` (m2, read side) | `totals.orphans_total += 0;` | new test RED, restore digest equal |

## Exit Criteria
- [ ] `cargo test -q -p pii-server --test live_orphan_is_countable 2>&1 | grep -qE "^test result: ok\. [1-9][0-9]* passed"`
- [ ] `test "$(grep -c '^#\[test\]' crates/pii-server/tests/live_orphan_is_countable.rs)" -ge 2`
- [ ] `! grep -q 'NodeObserved' crates/pii-server/tests/live_orphan_is_countable.rs && grep -q 'start_run' crates/pii-server/tests/live_orphan_is_countable.rs`
- [ ] `cargo test -q -p pii-server --test orphan_is_countable 2>&1 | grep -qE "^test result: ok\. 1 passed"`
- [ ] `bash scripts/checks/live-orphan-count.sh 2>&1 | tail -1 | grep -q "LIVE-ORPHAN-COUNT: OK"`
- [ ] `grep -q 'LIVE-ORPHAN-COUNT: OK' _docs/05-operations/LIVE-ORPHAN-COUNT.md`
- [ ] `git diff --quiet -- crates/pii-server/src crates/pii-graph/src crates/pii-server/tests/orphan_is_countable.rs crates/pii-server/tests/live_spawn_is_reaped.rs crates/pii-server/tests/spec_graph_daemon/mod.rs`
- [ ] `! grep -q '/projects/_tmp' scripts/checks/live-orphan-count.sh`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

## Files
- `crates/pii-server/tests/live_orphan_is_countable.rs`
- `scripts/checks/live-orphan-count.sh`
- `_docs/05-operations/LIVE-ORPHAN-COUNT.md`

## Guardrails
- **Do NOT seed the event.** No `RunEvent::NodeObserved { … }` construction in the new file, no copy of
  `orphan_is_countable.rs`'s `observed(...)` constructor, no reuse of `orphans_are_reaped.rs`'s `spawned(...)`. The
  evidence must be the journal a run wrote. If the harness cannot produce a hanging coder through `Engine::start_run`,
  **STOP and report** the measurement — a seeded fallback is the defect this spec removes.
- **Do NOT weaken, delete or `#[ignore]` an existing assertion**, and do not edit `orphan_is_countable.rs`,
  `live_spawn_is_reaped.rs`, `spec_graph_daemon/mod.rs` or anything under `crates/*/src`.
- **Do NOT `cp -p` a restore.** Write the bytes back, `touch`, and verify `sha256sum` before re-running: a stale-binary
  failure is not evidence that the mutation was reproduced.
- **Every wait has a deadline.** A poll loop that can spin forever, or a `sleep` with no bound, is a gate that hangs
  instead of a test that fails.
- **A genuine shape mismatch stops the run.** If the store a restarted engine reopens cannot report the run's terminal at
  all, or `recover_boot` no longer journals the reap, stop and report the measurement instead of forcing the assertion.
- **Do not commit.** Do not touch `specs/` beyond reading this file.
- **450 effective lines per file.** If the new test needs more than the gate allows, split it into a directory module the
  way `spec_graph_daemon/` is split, and say so.

## Closure
(pending — the orchestrator records the criteria runs, the terminal `recover_boot` left the run in, the header's
`process_start_ms`, both mutation reds with their restored digests, and the run's pid here after the item lands)

## Follow-ups
With the counter crossed, the same reading is owed where a per-run number is still folded from events a test wrote:
`pii run ls` / `cmd_trace`'s orphan and agent-call columns, and the `pii health` identity fields that read them, must be
shown to change when a real run's journal changes — a health number nobody has ever watched move on live data is the
same false green one seam further out.
