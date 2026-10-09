# Spec: 95 — bounded shutdown, and numbers before moving any more methods off the worker (N7)

**Project**: /projects/pi-improved
**Phase**: 045 re-review, finding N7 (ranked 11) — the last item. Two halves with different risk: one is a
clear defect, the other is speculative until measured. This spec fixes the first and MEASURES the second.

## Context
- **Defect (clear):** `pii-cli/src/cmd_daemon.rs:158` shuts the daemon runtime down without
  `shutdown_timeout`. Spec 80 moved heavy RPC (`memory.*`, `worker.reload`, `run.trace`, `session.prompt`)
  onto `spawn_blocking`, so an in-flight `memory.consolidate` or `worker.reload` now delays shutdown
  indefinitely rather than being cut off. The reviewer flagged this as N7.
- **Speculative (needs numbers):** these handlers are still dispatched INLINE by `engine/dispatch.rs`:
  `SessionCreate`, `RunCancel`, `RunStart`, `RunGet`, `RunList`, `ApproveResolve`, `SysReload`,
  `RunResume`, `SessionClose`, `GraphApply`, and `EventSubscribe`'s replay. Nobody has measured whether
  any of them can occupy the async worker long enough to matter. Moving them without numbers is
  speculation; leaving a genuinely slow one inline is a defect.

## Requirements
R1 (defect). Give daemon shutdown a BOUND. After the shutdown signal, use
    `runtime.shutdown_timeout(<bound>)`, with the bound a named constant and a one-line rationale
    (start at 5 s; say what drives the number). If the bound is hit, log it explicitly (the crate's
    existing `eprintln!("pii: …")` style) so a stuck drain is visible rather than silent. No busy-wait,
    no panic on timeout.
R2 (measurement). Produce a TABLE for the 11 handlers listed above: for each, the observed handler
    duration in a realistic harness, and whether it can block the async worker and by how much. Reuse
    spec 80's `current_thread` heartbeat harness (`crates/pii-server/src/engine/conn_heartbeat_tests.rs`)
    for the ones you can drive there; for any you cannot drive, say so explicitly rather than estimating.
    A method whose dependency must be stubbed slow: state what you stubbed and why that is realistic.
R3 (decision from numbers). Move a handler off the inline path ONLY if R2 shows it can occupy the worker
    beyond 50 ms (spec 80's bar) — and then it needs a heartbeat test in the spec-80 shape plus its own
    justification. If the numbers say none qualify, MOVE NOTHING and record that as the finding: the
    honest outcome here is a measured "they are fine inline".
R4 (control). The existing light-method control (`heartbeat_control_light_method_never_late`) must stay
    green throughout, and any new heartbeat test needs its own light-method control.

## Exit Criteria
- [ ] `grep -c 'shutdown_timeout' pii-cli/src/cmd_daemon.rs crates/pii-cli/src/cmd_daemon.rs 2>/dev/null | grep -v ':0' | wc -l` is ≥ 1
- [ ] a test proves the bound: with a deliberately stuck `spawn_blocking` task, daemon shutdown returns within the bound (+ a stated tolerance) and logs the timeout (record the test name and its output verbatim)
- [ ] the measurement table exists at `/projects/_tmp/spec95/INLINE-METHODS.md`, one row per handler, with duration, harness, and verdict (moved / stays inline + why)
- [ ] every handler left inline HAS a row saying so; every handler moved has a heartbeat test that fails when the method is put back inline
- [ ] no handler is moved without a number backing it
- [ ] `cargo test -p pii-server`, `cargo test -p pii-cli` pass
- [ ] `bash quality-gate.sh` exits 0 and `cargo run -q -p xtask -- fresh-gate --worktree` prints `fresh-gate: PASS`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` exits 0 and `cargo fmt --all --check` exits 0
- [ ] the report states plainly whether anything was moved, and if not, that the batch's last item is CLOSED as "measured, no change needed"

## Guardrails
- Do NOT change the response ORDER of RPC handling: spec 80's property (one frame at a time, response
  order preserved per connection) must survive any move.
- Do NOT make shutdown time out on tasks that would have finished in time — the bound must be generous
  enough for a normal in-flight call; say how you know.
- Do NOT delete or weaken any existing test, and do not touch `sched/` or the graph replay path.
- Scratch under `/projects/_tmp/`; delete your own exports (one fresh-gate export is ~8G).
- Do NOT commit; leave the tree for the orchestrator.

## Result (2026-10-09) — shutdown bounded, ONE handler moved on numbers, and the fresh-tree gate found #4
- **R1** `rt.shutdown_timeout(SHUTDOWN_DRAIN_BOUND)` (5 s) + a log line when the bound is hit, via a new
  `shutdown_runtime(rt, bound, log)` helper.
  **My mutation control**: restoring the pre-fix unbounded `drop(rt)` makes
  `shutdown_is_bounded_by_a_stuck_blocking_task` FAIL — `shutdown took 3.00028618s (bound 200ms)` — while
  its control (`shutdown_without_stuck_task_is_prompt_and_silent`) stays green. The fix is load-bearing.
- **R2/R3** All 11 handlers measured through the real `dispatch` (a real worker for `sys.reload`), table in
  `_tmp/spec95/INLINE-METHODS.md`. Exactly ONE moved: `event.subscribe` replay (2 ms for an 8-event
  journal, **102 ms for 5000 events** — it reads the whole journal, so it grows past the 50 ms bar).
  The other ten stay inline with their numbers recorded.
  **My mutation control**: putting `EventSubscribe` back inline makes `heartbeat_event_subscribe` FAIL —
  `event.subscribe blocked the async worker: tick late by 298.617834ms`; its control stays green.
- **Watch items recorded, not hidden:** `run.start` 20 ms idle but **53.8 ms under parallel gate load**
  (fsync-bound) — it crossed the bar under load and is left inline with that stated; `run.list` 29 ms at
  ~1000 stored runs; `sys.reload` shares `Supervisor::reload` with `worker.reload` and was measured with
  one healthy worker only (a slow/stuck worker could hold the worker thread — not driven, not estimated).
  The scale probes (5000 events, 1000 runs) are `#[ignore]`d; the timed test asserts only that calls
  succeed, because `run.start` is load-sensitive.

## Fresh-tree gate: FAILED, and it is right to fail
`cargo run -q -p xtask -- fresh-gate --worktree` → `failing tests:
["engine::runs::tests::stored_run_scope_root_recovers_from_journal",
"engine::runs::tests::stored_run_scope_root_none_when_unscoped"]`.

**Attribution (orchestrator-proven, not guessed):** this is a PRE-EXISTING latent defect of the N1 class,
exposed by the new tests changing intra-binary order. Decisive reproduction in the export:
- with `crates/pii-server/_tmp` PRESENT → both tests pass (and a full `cargo test --workspace --lib
  --bins --tests` in the export passes);
- with that directory *moved aside* → both FAIL at once:
  `runs.rs:472` and `runs.rs:516` panic `NotFound … crates/pii-server/_tmp/heal-scope-zHiSmL /
  heal-scope-vSuxNo` — a `tempdir_in(CARGO_MANIFEST_DIR/_tmp)` whose parent the tests never create.
They only survived spec 86's sweep because another test in the same binary creates the directory first
and happened to run before them. **Spec 96 fixes the class.**

So this spec's own criterion "`fresh-gate` prints PASS" is NOT met, for a reason that is now measured and
being fixed — recorded here rather than papered over, and the repo gate (its other criterion) is green.
