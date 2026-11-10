# pi-improved — spec-60: F-11 pilot — structured context for redb/store errors

- **Project**: `<repo>`
- **Priority**: LOW (pilot only) — Phase 5.3/5.4. F-11 as originally
  scoped ("357-site string-typed `PiiError`") is, per this session's
  own ground-truth count, actually **817 `PiiError::` references**
  across ~30 files — far too large for one spec or one pidag run. This
  spec is a deliberately bounded PILOT: establish a real, reusable
  pattern on ONE well-understood file family (redb/store errors in
  `pii-store`), NOT a workspace-wide rewrite. Treat this as Phase 1 of
  an N-phase effort; do not attempt more than this spec's explicit
  scope even if it looks mechanical to extend further — a bounded pilot
  that's fully verified is more valuable than a half-finished sweep.

## Overview

`PiiError::Internal(String)` and `PiiError::NotFound(String)` are used
throughout `pii-store` (and elsewhere) with ad-hoc `format!("heal: open
{}: {e}", path.display())`-style messages — human-readable, but not
machine-inspectable: nothing downstream can ask "was this a redb-open
failure or a redb-commit failure?" without string-matching the message,
which is exactly the class of fragility F-11 exists to fix.

This spec adds ONE new structured variant capturing "what redb
operation failed, on what resource" for the store layer specifically,
and converts `pii-store/src/heal.rs`'s 22 call sites (all `PiiError::
Internal(format!(...))` wrapping a redb operation) to use it, as the
concrete pilot. Ground truth: verify the exact current count and
message text yourself with `grep -n "PiiError::" crates/pii-store/src/
heal.rs` before starting — this spec's grounding is a snapshot, not a
guarantee it's unchanged.

## Requirements

### Functional

- R1. In `crates/pii-abi/src/error.rs`, add ONE new `PiiError` variant:
  ```rust
  /// A storage-backend (redb) operation failed on a specific resource.
  /// Structured so callers can inspect WHAT failed without parsing a
  /// message string (F-11 pilot).
  #[error("store: {op} on {resource}: {detail}")]
  StoreOp {
      /// The redb operation that failed (e.g. "open", "begin_write",
      /// "open_table", "commit", "insert", "get").
      op: String,
      /// What resource was involved (a table name, a file path, a key —
      /// whatever is most useful for that call site).
      resource: String,
      /// The underlying error's Display text.
      detail: String,
  },
  ```
  Read the existing `#[derive(...)]` list and `is_retryable`/
  `is_terminal`/`is_cancelled` methods on `PiiError` first — `StoreOp`
  should be classified the SAME way `Internal` currently is (neither
  retryable nor terminal; a redb failure is not something the caller
  should blindly retry, but it's also not necessarily permanent) unless
  reading the actual redb error types suggests otherwise. If you find a
  case where `StoreOp` SHOULD be retryable (e.g. a lock-contention
  error) that `Internal` currently isn't, flag it in your summary
  rather than silently changing the retry semantics of existing calls.
- R2. Convert all `PiiError::Internal(format!("heal: {op}: {e}", ...))`
  and `PiiError::Internal(format!("heal: {op} {resource}: {e}", ...))`
  call sites in `crates/pii-store/src/heal.rs` (all 22, per the ground-
  truth count above — re-verify the actual count first) to
  `PiiError::StoreOp { op: "...", resource: "...", detail:
  e.to_string() }`. Preserve the exact same information content the
  old message string carried — do not drop the path/table-name/key
  context that was in the format string, just restructure it into the
  three fields.
- R3. Existing tests in `heal.rs` (and anywhere else in the workspace
  that matches on `PiiError::Internal` specifically for a `heal.rs`-
  originated error, if any exist — grep for it) must be updated to
  match `PiiError::StoreOp` instead. Do NOT change test assertions'
  INTENT (what they're checking), only the pattern they match against.

### Non-Functional

- N1. Do NOT touch any file OTHER than `crates/pii-abi/src/error.rs`
  and `crates/pii-store/src/heal.rs` (plus whatever test files match
  on `heal.rs`'s errors specifically, per R3 — grep first, don't
  assume none exist).
- N2. Do NOT convert `PiiError::Internal` usages ANYWHERE ELSE in the
  workspace (not even other files in `pii-store` like `mem/`, `lib.rs`,
  `session.rs`, `approval.rs` — those are out of scope for this pilot;
  a future spec, if this pattern proves out, would extend it file by
  file).
- N3. `StoreOp`'s `Display` impl (the `#[error(...)]` string) must
  produce output at least as informative as the messages it replaces —
  spot-check a few converted call sites' old vs. new `.to_string()`
  output in your summary.

## Architecture

```
crates/pii-abi/src/error.rs   (+1 new PiiError::StoreOp variant)
crates/pii-store/src/heal.rs  (22 call sites converted; ~0 lines added,
                                same call count, different construction)
```

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| existing `heal.rs` tests (unchanged behavior, changed error pattern where relevant) | same as before | same pass/fail outcomes, now matching `StoreOp` |
| `store_op_display_carries_context` (new, in `error.rs`'s existing test module) | a `StoreOp { op: "commit", resource: "heal_attempt", detail: "disk full" }` | `.to_string()` contains all three of `commit`/`heal_attempt`/`disk full` |
| `store_op_retry_terminal_classification` (new, in `error.rs`'s existing test module) | a `StoreOp` instance | matches whatever classification R1 settled on (pinned like the existing `Internal`/`NotFound` pin tests already do) |

## Exit Criteria

- [ ] `cargo test -p pii-abi`
- [ ] `cargo test -p pii-store`
- [ ] `cargo test --workspace --lib --bins --tests`
- [ ] `cargo test --workspace --doc`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo fmt --all --check`
- [ ] `cargo run -q -p xtask -- silent-noop-check`
- [ ] `cargo run -q -p xtask -- lint-files`
- [ ] `! grep -n "PiiError::Internal" crates/pii-store/src/heal.rs`
- [ ] `grep -q "StoreOp" crates/pii-abi/src/error.rs`

## Guardrails

- G1. Do NOT touch any file other than `crates/pii-abi/src/error.rs`,
  `crates/pii-store/src/heal.rs`, and (only if R3 finds real matches)
  test files matching on `heal.rs`'s errors.
- G2. Do NOT remove or rename the existing `Internal`/`NotFound`/other
  variants — this is purely additive.
- G3. Do NOT touch `specs/` (read-only for the workhorse).
- G4. Do NOT commit — leave the diff in the working tree for the
  orchestrator to review and commit.
- G5. If the actual call-site count or shapes differ meaningfully from
  this spec's grounding (e.g. some sites don't cleanly decompose into
  op/resource/detail), STOP and report rather than forcing an awkward
  fit — this is a pilot meant to validate the pattern, not force it.
