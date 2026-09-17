# pi-improved — spec-46: move dead `HealAttempt` product fields into a generic `extra`

- **Project**: `/projects/pi-improved`
- **Priority**: MEDIUM — Phase 5.2, F-14 from the 026 report:
  `pii-store::heal` bakes product-specific fields into the generic
  persistence crate. **This is a real schema change to a live, persisted
  redb table, NOT a pure move-and-re-export refactor** like specs 43-45 —
  treat backward compatibility as the primary constraint.

## Overview

`HealAttempt` (`crates/pii-store/src/heal.rs`) is declared inside
`pii-store`, a crate the 026 report and this codebase's own layering intend
to be a **generic** persistence layer (see `crates/pii-store/src/lib.rs`'s
own module doc — read it before starting). But `HealAttempt` carries
product-specific fields that only the heal/self-repair FEATURE cares about:
`worktree`, `branch`, `patch_hash`, `model`, `tokens_in`, `tokens_out`,
`micro_usd`, `base_commit`. A generic store should not need to know what a
"worktree" or a "patch hash" is.

**Ground truth already confirmed by the orchestrator (do not re-derive,
verify instead)**:
- `patch_hash`, `model`, `tokens_in`, `tokens_out` ARE real: written by
  `HealStore::record_repair` (`heal.rs:280-309`), called from
  `crates/pii-server/src/nodes/heal.rs`'s `heal_record` node kind (reads
  `fix_node`'s `model`/`tokens_in`/`tokens_out` and `diff_node`'s patch
  text off the blackboard).
- `worktree`, `branch`, `base_commit`, `micro_usd` are declared, always
  initialized to `None`/`0` in both of `heal.rs`'s two struct-literal
  construction sites (`record_run`'s fallback at line ~231 and
  `record_decision`'s fallback at line ~330), and **never set to anything
  else anywhere in the workspace** — confirmed by `grep -rn
  '\.worktree\b|\.branch\b|\.base_commit\b|\.micro_usd\b' crates/` (only
  hits are the two initializer sites above, plus unrelated `pii-graph`
  test fixtures using `branch` as an expr-eval test key, and
  `pii-core::budget`'s own unrelated `BudgetKind::MicroUsd` variant — NOT
  a `HealAttempt.micro_usd` reference). **Re-run this grep yourself before
  writing code** — do not trust this spec's claim as gospel, the codebase
  may have changed.

## Requirements

### Functional

- R1. Add a new field to `HealAttempt`: `#[serde(default)] pub extra:
  serde_json::Map<String, serde_json::Value>` (or
  `serde_json::Value` defaulting to `Value::Object` — pick whichever
  serializes to an empty JSON object `{}` by default and is idiomatic
  serde_json usage; verify the choice compiles and round-trips through
  `to_bytes`/`from_bytes` before finalizing).
- R2. Remove the four genuinely-dead structural fields from `HealAttempt`:
  `worktree`, `branch`, `base_commit`, `micro_usd`. These carry no live
  data (R confirmed above) so there is nothing to migrate INTO `extra` for
  them — they simply disappear as named fields. If, on your own
  independent re-verification, you find these fields DO have a live
  writer this spec's ground-truth section missed, STOP and report it
  rather than silently deleting a field with real data (per the
  workspace's "when a spec's ground truth turns out wrong, report, don't
  silently work around" convention).
- R3. Keep `patch_hash`, `model`, `tokens_in`, `tokens_out` as named
  top-level `HealAttempt` fields, UNCHANGED — these have real, tested
  writers (`record_repair`) and readers (whatever consumes `HealStore` for
  display/reporting — grep for consumers before assuming there are none).
  Do NOT move these into `extra`; F-14's complaint is about
  *product-specific fields the generic store shouldn't need to know
  about*, and these four are already reasonably load-bearing to the heal
  feature's own tested behavior — moving them is unnecessary churn this
  spec does not ask for.
- R4. `HealStore::record_run` and `HealStore::record_decision`'s two
  struct-literal fallback constructions of a fresh `HealAttempt` (the
  "signature not seen before" case) drop the four removed fields and
  add `extra: serde_json::Map::new()` (or equivalent empty default).
- R5. Backward compatibility: an OLD serialized `HealAttempt` row (with
  the four now-removed fields present in its JSON/bincode encoding) must
  still deserialize without error. Check `crate::codec::{to_bytes,
  from_bytes}` (`pii-store/src/codec.rs`) to see which serialization
  format is actually used (the module doc says "encoded HealAttempt" —
  confirm exactly what "encoded" means: JSON via serde_json, or bincode,
  or something else) and add a regression test proving an old-shape
  payload (with `worktree`/`branch`/`base_commit`/`micro_usd` present)
  decodes cleanly into the new struct shape with `extra` empty. If the
  actual codec is bincode (position-dependent, not tolerant of removed
  fields — same class of bug as the `NodeStatus` vault corruption this
  session already diagnosed in `pidag`'s own FINDINGS.md), STOP and
  report rather than proceeding — that would mean this change breaks
  every existing heal-store row and needs a different strategy (e.g.
  keep the fields as `#[serde(skip_serializing)]` deprecated stubs
  instead of removing them outright). Do not assume JSON-like
  forward/backward tolerance without confirming the actual codec.

### Non-Functional

- N1. No new `unwrap()/expect()/panic!()/todo!()/unimplemented!()` outside
  `#[cfg(test)]` bodies.
- N2. `crates/pii-store/src/heal.rs` stays under the 450-line hard cap
  (currently 591 raw / verify effective count with `xtask lint-files`
  before and after — this spec should not need a further split, but
  check).

## Architecture

No new files. `HealAttempt`'s shape changes in place:

```rust
pub struct HealAttempt {
    pub signature_hash: String,
    pub gate: String,
    pub state: HealState,
    pub first_seen_ms: u64,
    pub last_seen_ms: u64,
    pub attempts: Vec<HealAttemptRun>,
    pub attempt_count: u32,
    pub patch_hash: Option<String>,      // kept — real writer
    // worktree, branch, base_commit REMOVED — dead, no writer
    pub gate_before: Option<String>,
    pub gate_after: Option<String>,
    pub decision: Option<ApprovalDecision>,
    pub model: Option<String>,           // kept — real writer
    pub tokens_in: u64,                  // kept — real writer
    pub tokens_out: u64,                 // kept — real writer
    // micro_usd REMOVED — dead, no writer
    #[serde(default)]
    pub extra: serde_json::Map<String, serde_json::Value>,  // NEW
}
```

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| `heal_attempt_extra_defaults_to_empty` | a fresh `HealAttempt` built via `record_run` on a new signature | `attempt.extra.is_empty()` |
| `heal_attempt_old_shape_deserializes_with_removed_fields_present` | a manually-constructed old-shape encoded payload (matching whatever `codec.rs` actually uses) containing `worktree`/`branch`/`base_commit`/`micro_usd` keys | decodes without error into the new `HealAttempt`, `extra` empty, no panic |
| (all existing tests in `heal.rs`'s `mod tests`, unchanged in assertions except any literal struct construction needing the field-list update) | — | still pass |

## Exit Criteria

- [ ] `! grep -n "pub worktree\|pub branch\|pub base_commit\|pub micro_usd" crates/pii-store/src/heal.rs`
- [ ] `grep -q "pub extra:" crates/pii-store/src/heal.rs`
- [ ] `cargo test -p pii-store --lib heal:: -- --nocapture 2>&1 | grep -qE "^test result: ok\. [1-9]"`
- [ ] `cargo test --workspace --lib --bins --tests`
- [ ] `cargo test --workspace --doc`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo fmt --all --check`
- [ ] `cargo run -q -p xtask -- silent-noop-check`
- [ ] `cargo run -q -p xtask -- lint-files`

## Guardrails

- G1. Do NOT change any EXISTING test's assertions or delete any existing
  test. Adding the two new tests above (or equivalent) is expected; the
  pre-existing `put_get_list_roundtrip`/`attempt_ring_caps_and_counts`
  tests keep their exact current assertions (update only the struct
  literal field list where the removed fields no longer compile).
- G2. Do NOT touch `crates/pii-server/src/nodes/heal.rs`'s
  `HealRecordFactory`/`HealRecordNode` unless the removed fields force a
  change there — per the ground-truth grep, they should NOT reference
  `worktree`/`branch`/`base_commit`/`micro_usd` at all, so no change
  should be needed; if one IS needed, report exactly why (this is your
  signal the ground-truth section was wrong).
- G3. Do NOT touch `specs/` (read-only for the workhorse).
- G4. Do NOT commit — leave the diff in the working tree for the
  orchestrator to review and commit.
- G5. If the actual redb encoding (`crate::codec::{to_bytes,from_bytes}`)
  turns out to be a format where removing a struct field breaks
  deserialization of old rows (e.g. positional bincode), STOP per R5 and
  report rather than proceeding with a change that corrupts every
  existing `HealStore` row on next read — this is the single highest-risk
  failure mode of this spec and must not be silently worked around.
- G6. No `unsafe` code (`#![forbid(unsafe_code)]`, workspace-wide).
