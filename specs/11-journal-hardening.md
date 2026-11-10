# Spec: Journal Hardening — bounded write-behind + interrupted-turn closer

**Project**: <repo>
**Phase**: post-3.6 — journal hardening (upgrade items 1+2 from the
DeepSeek-Harness comparison, 2026-09-06; see the workspace memory
`pi-improved/research/dsh-vs-pii-harness-comparison`)
**Status**: DRAFT — spec only, no implementation authorized yet. User
approved the direction ("so do it 1 and 2 the remaining worth a try?
document it and implement it with pidag!") — this spec is the
documentation step; implementation follows via `pidag sdd` with a
`deepseek/deepseek-v4-flash` worker.

## Overview

Two journal improvements, both transferable from DeepSeek Harness (dsh)
but adapted to pi's existing single-static-binary design:

1. **Bounded write-behind in the Journaler** — dsh batches session
   events in a bounded write window behind a `flush()` durability
   barrier, because per-event durable writes dominate cost. Pi appends
   every journal event in its OWN redb write transaction
   (`crates/pii-store/src/lib.rs:177 append_event` opens, writes,
   repoints the run-status bucket, and commits per call). Under bursty
   node chatter this is the dominant growth driver of the run journal
   — and a big share of the RSS slope that `xtask rss-gate` measures
   (the `SLOPE_KB=16384` threshold raise, commit `587c5d1`, is still
   flagged NEEDS-REVIEW; this is the legitimate fix that can retire
   it). Change: batch N events into one redb write transaction with an
   explicit flush barrier at the points where durability matters
   (terminal finish, suspend, status transitions), preserving the
   existing invariant that no redb transaction is ever held across a
   node dispatch.

2. **Interrupted-turn closer on resume** — dsh does NOT truncate or
   repair a crash-interrupted log; resume is the reader's job and it
   appends synthetic closers (missing tool errors, open step/end,
   synthetic `turn/end {reason: interrupted}`) so the model/operator
   can see WHY the previous attempt ended. Pi's resume
   (`crates/pii-graph/src/run.rs:124`) replays the journal, appends a
   bare `RunEvent::Resumed`, and continues. The dispatch code already
   journals `NodeStarted` BEFORE running a node
   (`crates/pii-graph/src/sched/dispatch.rs:193` — "a node interrupted
   mid-flight … resume re-runs it"), and `replay` already ignores a
   `NodeStarted` with no terminal event ("that node re-runs",
   `run_helpers.rs:60`). What is missing: the journal never RECORDS
   that the node was interrupted — a resumed run's trace shows the
   previous attempt's `NodeStarted` and then, inexplicably, a new
   `NodeStarted` for the same node with no terminal event in between.
   Change: on resume, detect nodes with a `NodeStarted` but no
   terminal event (`NodeFinished`/`NodeFailed`/`NodeCancelled`/
   `NodeDead`) in the journal and append a synthetic
   `RunEvent::NodeInterrupted` for each (in seq order, before the
   `Resumed` event), purely as an audit marker — `replay` treats it as
   a no-op for state (the node still re-runs, exactly as today).

## Requirements

### Functional

#### J1 — Store-level batched append (the transaction-count fix)
- Add `Store::append_events(&self, header: &mut RunHeader, events:
  &[RunEvent]) -> Result<u64, PiiError>` to `crates/pii-store/src/
  lib.rs`: one `begin_write` transaction, insert every event at
  `header.seq .. header.seq + events.len()`, persist the advanced
  header once, repoint the status bucket once, one `commit`. Return the
  first seq written (matching `append_event`'s return contract).
- `append_event` (single) becomes a thin wrapper over
  `append_events(header, &[event])` — no behavior change for existing
  callers, so the single-event path stays byte-identical.
- **Invariant (must not regress)**: no redb write transaction is ever
  held across a node dispatch — the batch is committed inside
  `append_events`, before it returns.

#### J2 — Journaler write-behind with a flush barrier
- `Journaler::append` (crates/pii-graph/src/journal.rs:119) keeps its
  exact public contract — returns the assigned seq, notifies the
  observer (`ObsEvent::Journal { seq, event }`) IMMEDIATELY at append
  time (the live TUI / `subscribe` bus reads the observer, NOT the
  store journal — journal writes may lag without any visible change),
  and advances the in-memory `header.seq` so `Journaler::seq()`
  reports the next-to-write seq exactly as today. The ONLY change is
  *when* events reach the store: they accumulate in a bounded pending
  buffer and are flushed in one `append_events` call.
- Flush triggers (any of):
  - pending count reaches the batch threshold (default `16`; make it a
    `Journaler` constructor parameter so tests can force flush-heavy
    or batch-heavy behavior — `fresh`/`adopt` get a `with_batch` style
    variant or an explicit field; do NOT invent a config file knob for
    this in v1),
  - `set_status()` is called (a status transition must be durable
    before the event that carries it — flush the pending buffer first
    so the header persisted in the same transaction as the final batch
    carries the new status),
  - `snapshot_now()` (suspend path) — flush first so the snapshot seq
    matches a durable journal prefix,
  - the run reaches a terminal disposition (`Engine::finish` appends
    `RunEvent::Finished` — it MUST flush after that append so a
    completed run's journal is fully durable the moment `run()`/
    `resume()` returns, which is what every post-run journal reader
    (`cmd_trace`, the tests in pii-graph/tests/{resume,diamond,
    loops}.rs, `reattach`, `bus.rs`, `daemon/mod.rs`) already depends
    on),
  - drop of the Journaler (defensive last flush, best-effort).
- **Crash semantics**: a process killed mid-run with events still in
  the pending buffer loses exactly those unflushed events — the journal
  simply ends at the last flushed prefix (still a valid contiguous
  log). This is the dsh model ("best-effort append; only flush promises
  durability") and is strictly better than today only in that far fewer
  transactions were opened. The header persisted by the last flush
  still carries `status: Running`, so recovery (`spawn_resume` on boot,
  `crates/pii-server/src/daemon/scheduler.rs:90`) behaves exactly as
  today. This is a deliberate, documented trade — do NOT flush on
  cancellation (the `PiiError::Cancelled` return path in
  `Engine::tick_loop`) — a cancelled run is precisely the case where
  durability does not matter and the whole point of batching is not
  spending transactions on runs that never finish.
- `snapshot_now` seq semantics: after a flush, `last_seq` equals the
  durable prefix length; the blackboard snapshot is anchored there —
  unchanged from today's contract.

#### J3 — New `RunEvent::NodeInterrupted` + resume detection
- Add `RunEvent::NodeInterrupted { node: NodeId }` to the enum in
  `crates/pii-abi/src/journal.rs`. This is a journal/audit marker; it
  carries NO state transition. Concretely:
  - `replay()` (run_helpers.rs) ignores it (falls into the `_ => {}`
    arm) — a node with `NodeInterrupted` and no terminal event still
    re-runs on resume, byte-identical to today,
  - the journal schema is additive; old journals replay fine, and any
    exhaustive `match` over `RunEvent` in the workspace gains the new
    arm (compile error will enumerate the sites — expected and
    required).
- `resume()` (run.rs), between reading the journal and appending
  `Resumed`: compute the set of nodes whose last relevant event is
  `NodeStarted` with NO later terminal event
  (`NodeFinished`/`NodeFailed`/`NodeCancelled`/`NodeDead`) anywhere in
  the journal, and append `NodeInterrupted { node }` for each, ordered
  by the seq of their last `NodeStarted` (deterministic). Only then
  append `Resumed`. Rationale: the operator/trace/TUI sees
  `NodeStarted -> NodeInterrupted -> Resumed -> NodeStarted`, i.e. the
  previous attempt's fate is explicit instead of an unexplained
  duplicate `NodeStarted`.
- A run whose journal has no dangling `NodeStarted` (clean suspend
  resume, normal resume) appends NO `NodeInterrupted` events — the
  journal is unchanged from today in the common case.
- The CLI trace pretty-printer (`pii graph trace`, cmd_trace.rs) should
  render the new variant with a human line (e.g. `node <id> interrupted
  (resume)`) — grep how it formats the other node events and follow
  suit; do not redesign the trace output.
- `pii tui`'s event log maps `Event`/`LogLine` — check whether the new
  `RunEvent` variant surfaces there; if the TUI only consumes a subset
  (bus events, not raw RunEvent), no change needed beyond what the
  compiler demands.

#### J4 — rss-gate legitimacy (the payoff)
- After J1+J2 land, re-run the FULL `xtask rss-gate --secs 300
  --interval 10 --max-mb 50 --runs 3000` and compare `slope_kb`
  against the `16384` threshold and against the pre-change baseline
  (~13.8 MB slope). Target: slope materially lower than baseline. If it
  is, that is the evidence to propose retiring the `SLOPE_KB=16384`
  flagged-for-review threshold change (commit `587c5d1`) back toward
  `2048` or a middle value — but ONLY propose; the threshold review is
  the user's call, and the rss-gate itself must stay green on the
  current threshold either way.
  - NOTE for the pidag run: the full 300s gate takes ~6 minutes and
    would be re-run on every validate cycle — per the pidag skill's
    warning, do NOT put the full gate in the machine-checked exit
    criteria; use a SHORT probe (`--secs 60 --interval 5 --runs 2000`,
    ~1 min) in the EC list and run the full 300s form ONCE manually at
    the very end.

### Non-Functional
- No change to the journal read API, the run status bucket semantics,
  the observer event stream, the seq numbering contract, or the
  content-addressing of inputs/outputs — all existing tests in
  pii-graph/tests/{resume,diamond,loops}.rs, pii-store/tests/
  store_contract.rs, and pii-server/tests/reattach.rs pin these and
  must pass UNMODIFIED except where a test deliberately asserts
  per-event transaction counts (there are none today — J1 is invisible
  to them).
- Line-count lint (450/file) is in force; journal.rs is 160 lines
  today and will grow — extract the pending-buffer bookkeeping into
  `crates/pii-graph/src/journal.rs` private helpers or a small
  `journal_buf` module if needed; run.rs is 466 lines (already over the
  warn threshold) — put the interrupted-detection helper in
  `run_helpers.rs` (185 lines) or a new small module, NOT in run.rs.

## Architecture

```
Today:  append() -> Store::append_event -> [begin_write | insert 1 event |
        rewrite header | repoint status | commit]        (per event!)

J1+J2:  append() -> buffer (seq assigned, observer notified, header.seq
                     advanced in memory)
          |  flush triggers: batch full | set_status | snapshot_now |
          |                   terminal finish | drop
          v
        Store::append_events -> [begin_write | insert N events |
        rewrite header once | repoint status once | commit]   (per batch)

J3 (resume):  read_journal -> detect dangling NodeStarted (no terminal)
        -> append NodeInterrupted{node} per node (seq order)
        -> append Resumed -> replay -> tick_loop (node re-runs as today)
```

The anchor test for J3 already exists and must keep passing:
`resume_never_reinvokes_completed_node` (crates/pii-graph/tests/
resume.rs:67) cancels a run mid-`wait` (node `b` left as dangling
`NodeStarted`), resumes, and asserts `a` runs exactly once — with J3
the journal additionally carries a `NodeInterrupted { node: "b" }`
before `Resumed`, and the test's assertions must still hold (extend it
to also assert the new event is present exactly once for `b`).

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| `append_events_batches_in_one_transaction` (pii-store unit) | `Store::append_events` with 5 events | all 5 present in `read_journal` with consecutive seqs; header.seq advanced by 5; status bucket correct; a transaction-count probe (see below) shows 1 write txn, not 5 |
| `append_event_single_is_unchanged` (pii-store unit) | existing single-event path | byte-identical journal/header behavior to pre-change (the existing `store_contract.rs` tests already assert this — they must pass unmodified) |
| `journaler_flushes_on_threshold` (pii-graph unit, batch=2) | append 3 events, no explicit flush | after 2 appends the store journal has 2 events (flush fired); after the 3rd it still has 2 (buffered); observer received all 3 with correct seqs |
| `journaler_flushes_on_set_status` | batch=64, set_status then append | status change is durable before the appending event's batch — store header shows the new status |
| `finished_run_journal_is_fully_durable` | run a real graph to Success with batch=1_000_000 (forces no mid-run flush) | after `run()` returns, `read_journal` contains every event incl. `Finished` (the terminal flush fired) |
| `cancelled_run_loses_only_unflushed_events` | batch=1_000_000, cancel mid-run (existing crash-resume test shape) | journal is a valid contiguous prefix; resume still works and re-runs the interrupted node |
| `resume_appends_interrupted_closer_for_dangling_node` (extend resume.rs:67) | cancel mid-`wait`, resume | journal contains exactly one `NodeInterrupted { "b" }` ordered before `Resumed`; `a` still finished exactly once; terminal Success |
| `resume_appends_no_interrupted_without_dangling_node` | a clean suspend->resume run | journal has NO `NodeInterrupted` events (common case unchanged) |
| `replay_ignores_node_interrupted` (pii-graph unit) | journal with `NodeStarted`+`NodeInterrupted` and no terminal for node X | replay leaves X re-runnable (not in executed/dead/failed/cancelled), exactly as if `NodeInterrupted` were absent |

Transaction-count probe: expose (in pii-store, `#[cfg(test)]` or a
`#[must_use]` counter) a monotonic `write_txn_count()` on Store (an
atomic bumped in `begin_write`) so tests can assert 1-vs-5 without
strace. This is test instrumentation only — not a public API promise.

## Exit Criteria
- [ ] `cargo test -p pii-store` — all pass (incl. unmodified
      `store_contract.rs`)
- [ ] `cargo test -p pii-graph` — all pass (incl. unmodified
      `tests/resume.rs` `tests/diamond.rs` `tests/loops.rs` + new
      journal-hardening tests)
- [ ] `cargo test --workspace` — 0 failures
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` — 0 errors
- [ ] `cargo fmt --all --check` — clean
- [ ] SHORT rss probe: `xtask rss-gate --secs 60 --interval 5 --max-mb
      50 --runs 2000` — PASS on the current threshold
- [ ] Manual, AFTER everything above is green: full
      `xtask rss-gate --secs 300 --interval 10 --max-mb 50 --runs 3000`
      — PASS, and record `slope_kb` for the threshold-review proposal
- [ ] `pii graph trace` on a resumed-interrupted run shows the
      `interrupted` line for the dangling node (manual CLI check)

## Guardrails
- Do NOT flush on the `Cancelled` path — durability does not matter for
  a run that never finishes, and flushing there throws away the entire
  point of batching. (dsh's own model: "best-effort append; flush is
  the only durability promise.")
- Do NOT change the single-event `append_event` observable behavior —
  J1 is purely additive batching underneath; every existing
  post-run journal reader must pass unmodified.
- Do NOT let `NodeInterrupted` become a state transition in `replay` —
  it is an audit marker; the node re-runs exactly as it does today.
  If a future spec wants resume to SKIP a previously-interrupted node,
  that is a separate, deliberate behavior change with its own spec.
- Do NOT put the full 300s rss-gate in the machine-checked exit
  criteria (pidag re-runs ECs on every validate cycle — a 6-minute
  gate would add 6+ minutes per iteration). Short probe in the ECs,
  full gate once manually at the end.
- Do NOT touch the threshold constant in `xtask/src/rss_gate.rs` in
  this spec — J4 only produces evidence for the (user-owned) review of
  the `587c5d1` change.
- Do NOT add a config file / env knob for the batch size — a
  constructor parameter on `Journaler` suffices; config plumbing is
  out of scope.
