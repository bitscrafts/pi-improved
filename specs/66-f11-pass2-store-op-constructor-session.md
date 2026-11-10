# F-11 Pass 2: PiiError::store_op Constructor, then session.rs

- **Project**: `<repo>`
- **Crate**: `pii-abi`, `pii-store`
- **Priority**: MEDIUM — continues the F-11 structured-error sweep
  (pilot: commit `c8fb4d9`, `pii-store/src/heal.rs`). Sourced from
  fable review finding 4 / candidate #1 (both passes agreed on this),
  `_docs/06-research/032-fable-pii-review-2026-09-18.md`.

## Overview

The F-11 pilot converted 15 `PiiError::Internal(format!(...))` sites
in `heal.rs` to the literal struct form `PiiError::StoreOp { op:
String::from(...), resource: String::from(...), detail: format!(...) }`
— 5 lines per site against the original 1. That worked for 15 sites
(+82 net lines). `session.rs` has 70 `PiiError::Internal` call sites
today (measured now — fable's review measured 58 at the time; the
file has grown since, verify the current count yourself before
starting). Applying the pilot's literal form to all of them would add
roughly 280+ effective lines, very likely pushing `session.rs` over
the 450-line gate (`xtask lint-files`, see the companion spec-67 for
the counting mechanics).

The fix identified by review: add a one-line constructor so every call
site becomes a single expression instead of a 5-line struct literal.

Ground truth (verify yourself, re-read both files — line numbers will
have moved since this spec was drafted):
- `crates/pii-abi/src/error.rs` — the `StoreOp` variant's exact field
  shape (`op: String`, `resource: String`, `detail: String`) and its
  existing tests (`store_op_display_carries_context`,
  `store_op_retry_terminal_classification`) as the pattern to extend,
  not replace.
- `crates/pii-store/src/heal.rs` — the 15 already-converted call
  sites, as the reference for what `op`/`resource` values look like in
  practice (they come from the existing `format!("store: {op}
  {resource}: {e}")`-shaped strings that predate the pilot).
- `crates/pii-store/src/session.rs` — every `PiiError::Internal(...)`
  site; almost all are `.map_err(|e| PiiError::Internal(format!("store:
  <op> <resource>: {e}")))` in the same shape `heal.rs` had
  pre-conversion; a small number (verify count, fable's review found 2
  in the version it read) are fault-injection test-only returns, not
  real error paths — do not convert those, they're deliberately
  distinct from a real store-operation failure and converting them
  would be misleading. Read each site individually; do not
  find-and-replace blindly.

## Requirements

### Functional

- R1. Add to `crates/pii-abi/src/error.rs`:
  ```rust
  impl PiiError {
      /// Convenience constructor for `StoreOp` — turns the common
      /// `.map_err(|e| PiiError::Internal(format!("store: {op}
      /// {resource}: {e}")))` shape into a one-line call. `detail`
      /// accepts anything `Display`, so both `impl std::fmt::Display`
      /// errors and pre-formatted strings work without an extra
      /// `.to_string()` at call sites.
      pub fn store_op(op: &str, resource: &str, detail: impl std::fmt::Display) -> Self {
          PiiError::StoreOp {
              op: op.to_string(),
              resource: resource.to_string(),
              detail: detail.to_string(),
          }
      }
  }
  ```
  Add one test (`store_op_constructor_matches_manual_construction` or
  similar) asserting `PiiError::store_op("open", "heal_db", "boom")`
  produces output identical to the manual struct-literal form.
- R2. Retrofit the 15 existing sites in `crates/pii-store/src/heal.rs`
  (from commit `c8fb4d9`) to use the new constructor instead of the
  literal struct form — this should net-shrink `heal.rs` by
  approximately 60 effective lines (per fable's review estimate;
  confirm the actual delta after the change).
- R3. Convert every genuine store-operation-failure
  `PiiError::Internal(...)` call site in `crates/pii-store/src/session.rs`
  to `PiiError::store_op(op, resource, e)` (or `PiiError::store_op(op,
  resource, format!(...))` if the detail needs more than the bare
  error). Derive `op`/`resource` from each site's existing format
  string — do not invent new terminology, reuse what the string
  already says. Leave the fault-injection test-only returns (if any
  are confirmed genuinely distinct from real error paths) as
  `PiiError::Internal` — do not convert those.
- R4. Update `session.rs`'s `# Errors` doc comments (if any function
  docs mention `PiiError::Internal` specifically) to say `StoreOp`
  instead, so the docs match the actual returned variant.

### Non-Functional

- N1. `PiiError::StoreOp`'s classification (`is_retryable()`/
  `is_terminal()`, both false, matching `Internal`'s behavior) must be
  completely unchanged — this spec only changes HOW the variant is
  constructed at call sites, not what it means.
- N2. `session.rs`'s effective line count (per `xtask lint-files`,
  NOT raw `wc -l`) must stay under 450 after this change — the whole
  point of the constructor is to keep the conversion cheap. If it
  still approaches or exceeds 450 despite using the constructor, STOP
  and report rather than splitting the file as an unplanned side
  effect (a split, if needed, is a separate decision).
- N3. No code outside `pii-abi` pattern-matches on `PiiError::Internal`
  specifically in a way that would break when a site switches to
  `StoreOp` (fable's review confirmed this for the sites it converted;
  re-verify for `session.rs`'s specific call sites before assuming
  it's still true).

## Architecture

Described inline in Overview above (file paths and function names given there constitute the architecture context for this change).

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| `store_op_constructor_matches_manual_construction` | `PiiError::store_op("open", "heal_db", "boom")` vs. manual `PiiError::StoreOp { op: "open".into(), resource: "heal_db".into(), detail: "boom".into() }` | identical `Display` output, identical `is_retryable()`/`is_terminal()` |
| `session_rs_has_no_internal_variant_for_store_ops` | `grep -c "PiiError::Internal(" session.rs` after conversion | 0 (or the exact count of genuinely-distinct fault-injection sites, if any remain — state that number explicitly in your report) |
| `heal_rs_retrofit_still_passes_existing_tests` | `cargo test -p pii-store -- heal` | all pre-existing heal.rs tests still pass unchanged |
| `session_rs_conversion_still_passes_existing_tests` | `cargo test -p pii-store -- session` | all pre-existing session.rs tests still pass unchanged |

## Exit Criteria

- [ ] `grep -q "pub fn store_op" crates/pii-abi/src/error.rs`
- [ ] `cargo run -q -p xtask -- lint-files` (confirms session.rs stays under the effective-line cap after conversion)
- [ ] `cargo test -p pii-abi -p pii-store -p pii-server`
- [ ] `cargo build -p pii-abi -p pii-store`
- [ ] `cargo clippy -p pii-abi -p pii-store -- -D warnings`
- [ ] `cargo fmt --check`

## Guardrails

- Do NOT touch `crates/pii-store/src/mem/read.rs`, `mem/write.rs`,
  `mem/mod.rs`, `timer.rs`, `channel.rs`, `approval.rs`, or `lib.rs`
  (other files fable's review flagged with `PiiError::Internal` sites)
  — this spec is scoped to the constructor plus `heal.rs`/`session.rs`
  only. Those other files are candidates for a LATER pass once this
  one lands cleanly.
- Do NOT convert fault-injection test-only `PiiError::Internal`
  returns that are deliberately distinct from real store-operation
  failures — read each site's context before converting.
- Do NOT modify any file under `specs/`.
- No emojis, no AI-tool mentions in any comment or commit message.
- If `session.rs`'s post-conversion effective line count exceeds 450
  despite the constructor, STOP and report the exact count rather than
  splitting the file to compensate — that's a separate decision.
