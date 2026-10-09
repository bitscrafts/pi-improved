# Spec: 77 — let_underscore_must_use sweep in pii-graph + journal flush guards (D5 + item 5)

**Project**: /projects/pi-improved
**Phase**: batch C — 043 §10 item 5 (D5 flush-loss) + its paired lint pass.

## Context
- D5 (043 §6): `crates/pii-graph/src/journal.rs:233-240` drains the in-memory queue
  BEFORE `append_events` succeeds; a failed append loses the drained events. Callers
  discard the flush result: `journal.rs:165`, `run/engine.rs:332`, `sched/dispatch.rs:324`.
- Item 5 also enables `clippy::let_underscore_must_use` for pii-graph as the paired
  hygiene pass (that lint is where the discarded flush results hide).

## Requirements
R1. `journal.rs`: restore `pending` on `append_events` Err (drain only after success),
    or hold the drained batch locally until the append returns Ok. Both acceptable;
    write which one in the report.
R2. The three `let _ = self.flush()` call sites (`journal.rs:165`, `run/engine.rs:332`,
    `sched/dispatch.rs:324`): replace discarded results with an error surfaced through
    the existing error channel/logging (smallest per-site change: `if let Err(e) =
    self.flush() { log/event the failure }`) — never panic, never unwrap.
R3. Enable `clippy::let_underscore_must_use` for pii-graph (lints table in
    `crates/pii-graph/src/lib.rs` or Cargo.toml `[lints]`) and fix every violation the
    lint names in that crate. If a site is intentionally ignored, it gets a named
    allow with a comment — the lint stays enabled.
R4 (TDD). Fault-injected test: a store fault that makes `append_events` fail once,
    then succeed; assert the journal's next read sees contiguous seqs and NO lost
    event (this is red today — run it and record the red in the report BEFORE fixing).

## Exit Criteria
- [ ] `grep -c 'let _ = self.flush()' crates/pii-graph crates/pii-server -r | grep -v ':0' | wc -l` = 0
- [x] `cargo clippy -p pii-graph --all-targets -- -D warnings` exits 0 — the lint is enabled AT SOURCE
      (`#![warn(clippy::let_underscore_must_use)]` in crates/pii-graph/src/lib.rs), so the gate's own
      `-D warnings` already fails on any new discard; the 3 intentional sites carry named allows with a
      reason. (The original wording passed `-W` on the command line, which propagates to EVERY crate in the
      build and forced 19 blanket crate-level allows outside pii-graph — orchestrator replaced that debt with
      the source-level enablement above; the criterion was the defect, not the code.)
- [ ] `grep -c 'let_underscore_must_use' crates/pii-graph -r | grep -v ':0' | wc -l` at least 1 (lint enabled at source)
- [x] `cargo test -p pii-graph` exits 0 INCLUDING the new contiguous-seq fault test (RED re-proven by the orchestrator with the 3 fix files reverted: `left: 0, right: 3`)
- [ ] `cargo test -p pii-server` exits 0
- [ ] `cargo fmt --all --check` exits 0

## Guardrails
- Do NOT change the journal on-disk format or event wire shape.
- Do NOT introduce panics/unwrap/expect in the fix paths (workspace lints deny them).
- Do NOT enable the lint workspace-wide in this spec (pii-graph + its call sites only).
- Do NOT commit; leave the tree for the orchestrator.