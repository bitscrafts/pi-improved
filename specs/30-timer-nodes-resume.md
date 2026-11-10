# pi-improved — spec-30: timer-nodes-resume

- **Project**: `<repo>`
- **Priority**: BLOCKER for the entire boss loop. `TimerNode::run`
  (`crates/pii-graph/src/nodes.rs:292-311`) has NO code path returning
  `Outcome::Next` — every dispatch unconditionally returns
  `Outcome::Suspend { reason: Timer, key }`. `pii_graph::run::resume`
  (`crates/pii-graph/src/run.rs:124-160`) replays the journal, finds no
  `NodeFinished` for the timer node, re-dispatches it, and it suspends again
  with a NEW `fire_at_ms`. A `timer`-entry graph (`boss-watch.toml`,
  `heal-watch.toml`) therefore never progresses past its first suspension.
  The shipped boss detects nothing. This is documented honestly at
  `crates/pii-server/tests/boss_watch.rs:7-28`. Nothing else in P4.7 is worth
  building until this is fixed.

## Verified Premises

- `TimerNode::run` returns only `Outcome::Suspend` (verified above).
- `pii_graph::run::resume` re-dispatches a suspended node with no
  `NodeFinished` (verified `run.rs:124-160`).
- `GraphNode` trait has only `fn run` — no resume hook
  (`crates/pii-graph/src/node.rs:10-16`).
- `SuspendReason::Timer` and the `ResumeKey` format
  (`nodes.rs:300-305`) feed the `TIMER_WHEEL` index
  (`crates/pii-store/src/timer.rs`).
- `gate` suspension already round-trips correctly via `gate.rs:83-87` — do
  not touch it.
- `CronSection` (`crates/pii-server/src/daemon/config.rs:167-178`) is unused
  by any deployed config; there is no `[[cron]]` entry for `boss-watch`
  anywhere.

## Overview

Make a suspended `timer` node whose wheel entry has fired resume to
`Outcome::Next { port: "fired" }` instead of re-suspending, and wire a
`[[cron]]` entry so periodicity comes from the daemon scheduler rather than an
external `pii-evolve` invocation. This unblocks the shipped `boss-watch` and
`heal-watch` graphs.

## Requirements

### Functional

- **R1 (timer resumes to fired)**: a suspended `timer` node whose wheel entry
  has fired resumes to `Outcome::Next { port: "fired" }` instead of
  re-suspending.
- **R2 (heal-watch fixed by same change)**: `heal-watch.toml` is fixed by the
  same change — no per-graph special case.
- **R3 (once-per-fire)**: replaying a completed timer does not re-dispatch it.
- **R4 (trace shows it)**: `pii graph trace <run_id>` shows
  `NodeStarted`/`NodeFinished` for the timer node.
- **R5 (scheduler periodicity)**: a `[[cron]]` entry for `starter/boss-watch`
  in the starter config, wired through `CronSection`
  (`config.rs:167-178`), so periodicity comes from the scheduler rather than
  from an external `pii-evolve` invocation.

### Non-Functional

- **N1**: the quality gate stays green; the test count only goes up.
- **N2**: no new third-party dependencies.
- **N3**: `SuspendReason::Timer` and the `ResumeKey` format stay byte-identical
  so the `TIMER_WHEEL` index keeps working.
- **N4**: `silent-noop-check` (spec 24) stays green.

## Architecture

**Design decision (D1 from opus plan 012):** add `fn on_resume(&self, ctx,
input, key) -> Option<Outcome>` to the `GraphNode` trait
(`crates/pii-graph/src/node.rs:10-16`), default `None`, and have `run::resume`
pass the matched `ResumeKey`. This is clean and general (a future
`wait`/webhook node needs it too), and it keeps node semantics in the node
rather than hiding control flow in the scheduler replay path. The ABI is
pre-1.0, so the trait change is acceptable.

```
run::resume (journal replay)
  → node has NodeFinished? → skip (R3)
  → node suspended with ResumeKey K
  → K matches a fired wheel entry?
      → yes → node.on_resume(ctx, input, K) → Some(Outcome::Next{port:"fired"})
      → no  → re-dispatch node.run() (current behavior)
```

`TimerNode::on_resume` returns `Some(Outcome::Next { port: "fired", value })`
when the key's `fire_at_ms` has passed; otherwise `None` (re-suspend).

## TDD Contract

| Test | Input | Expected Output |
|---|---|---|
| `timer_resumes_to_fired_port` (T1, acceptance) | `timer(in_ms=1) → transform` graph; run, then resume via `runs::spawn_resume` after the fire time | terminal is `Done`, and the journal contains exactly one `NodeFinished{node:"tick", port:"fired"}` — NOT a second `Suspended` |
| `boss_watch_reaches_detect_after_resume` | the REAL `graphs/starter/boss-watch.toml` via `Engine::load_spec`, resumed once | journal contains `NodeStarted{node:"check"}` |
| `timer_does_not_refire_on_replay` | resume a run whose timer already finished | timer dispatch count stays 1 (assert the count, not the terminal) |
| `cron_section_wires_boss_watch` | starter config with `[[cron]]` for `starter/boss-watch` | `CronSection` parses it; the scheduler has a scheduled entry for it |

T1 must FAIL if the fix is reverted — today it produces a second
`Terminal::Suspended`.

## Exit Criteria

Machine-checked (each >=1 test, non-vacuity `grep -qE "ok\\. [1-9]"`):

- [ ] `cargo test -p pii-graph timer_ 2>&1 | grep -qE "test result: ok\. [1-9]"` — timer resume tests (R1, R3)
- [ ] `cargo test -p pii-server boss_watch_ 2>&1 | grep -qE "test result: ok\. [1-9]"` — real-graph resume test (R2, R4)
- [ ] `cargo test -p pii-server cron_ 2>&1 | grep -qE "test result: ok\. [1-9]"` — CronSection wiring (R5)
- [ ] `cargo test --workspace` — 0 failures (N1)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` — 0 errors (N1)
- [ ] `cargo fmt --all --check` — clean (N1)
- [ ] `cargo xtask silent-noop-check` — green (N4)

## Guardrails

- **G1 — do NOT edit this spec.**
- **G2 — do NOT commit.** Leave work in the working tree for orchestrator review.
- **G3 — do NOT weaken, skip or delete an existing test.**
- **G4 — no new node kind.** The fix is a trait method + `TimerNode` impl, not
  a new node.
- **G5 — do NOT touch `gate` suspension** (it already round-trips correctly).
- **G6 — `SuspendReason::Timer` and the `ResumeKey` format stay byte-identical.**

## Files to Modify

- `crates/pii-graph/src/node.rs` — add `on_resume` to the `GraphNode` trait
  (default `None`).
- `crates/pii-graph/src/nodes.rs` — implement `TimerNode::on_resume`.
- `crates/pii-graph/src/run.rs` — `resume` passes the matched `ResumeKey` to
  `on_resume`.
- `crates/pii-server/src/daemon/config.rs` — wire `CronSection` (R5).
- `crates/pii-server/src/daemon/scheduler.rs` — schedule `[[cron]]` entries.
- `graphs/starter/boss-watch.toml` — add `[[cron]]` entry (R5).
- Tests: `crates/pii-graph/tests/` or `crates/pii-server/tests/` for the TDD
  contract.
