# pi-improved — spec-55: pii-proto JSON round-trip tests (F-18)

- **Project**: `/projects/pi-improved`
- **Priority**: MEDIUM — Phase 5.3, F-18: `pii-proto` has zero tests
  (no `tests/` dir, and only `event.rs`/`lib.rs` have inline
  `#[cfg(test)]`; `client.rs`, `method.rs`, `run.rs`, `session.rs`,
  `sys.rs` have none). This spec covers `method.rs`, `run.rs`,
  `session.rs`, `sys.rs` — the pure serde wire types with no I/O.
  `client.rs` (270 lines, needs a live UDS server/mock to exercise
  meaningfully) is explicitly OUT of scope for this spec.

## Overview

Every wire type in `method.rs`/`run.rs`/`session.rs`/`sys.rs` derives
`Serialize + Deserialize + PartialEq` already — none has ever been
round-tripped through actual JSON. Add a test file that serializes each
type to JSON, deserializes it back, and asserts equality — catching the
class of bug where a `#[serde(rename = "...")]`/`#[serde(tag = "...")]`
attribute silently breaks wire compatibility with no test noticing
until a real client/server mismatch in production.

Ground truth already confirmed by the orchestrator (verify field names/
variants yourself from the actual source before writing tests — this
list is a starting map, not a substitute for reading the files):

- `method.rs`: `Method` enum — closed wire-method vocabulary (F1.3), one
  variant per `sys.*`/`session.*`/`run.*`/`graph.*` method, each with an
  explicit `#[serde(rename = "...")]` matching the exact wire string
  (e.g. `Method::SysHello` ↔ `"sys.hello"`). The whole point of this
  type is the wire string never silently drifts from the Rust variant
  name — that's exactly what a round-trip test proves.
- `run.rs`: `WireRunStatus`, `RunStartParams`, `RunStartResult`,
  `WireRunHeader`, `RunListParams`, `SeqEvent` (wraps `crate::event::
  Event`, which DOES already have its own tests — reuse a minimal
  `Event` variant, don't re-test `Event` itself here), `TraceResult`,
  `GraphApplyParams`, `GraphApplyResult`, `GraphLintParams`,
  `GraphLintResult`.
- `session.rs`: `ContentBlock` (tagged enum, `#[serde(tag = "type")]`),
  `SessionStatus`, `SessionCreateParams` — read the rest of the file
  (108 lines total) for anything past what's summarized here.
- `sys.rs`: `PeerInfo`, `HelloParams`, `Limits`, `HelloResult`,
  `WorkerStatus`, `ListenerStatus`, `ChannelStatus`, `MemoryStatus`,
  `StatusResult`.

## Requirements

### Functional

- R1. New file `crates/pii-proto/tests/wire_roundtrip.rs`. One test per
  distinct type listed above (not per module) — construct a
  representative instance with realistic field values (not all-zero/
  all-empty defaults; e.g. a `RunListParams` with `status: Some(...)`
  AND a separate case/assertion for `status: None`, since `Option`
  fields are exactly where round-trip bugs hide), serialize to a JSON
  string via `serde_json::to_string`, deserialize back via
  `serde_json::from_str`, assert the result equals the original
  (`PartialEq`, already derived on every type).
- R2. For `Method`: additionally assert the LITERAL wire string for
  every variant matches its doc'd `#[serde(rename = "...")]` value —
  e.g. `serde_json::to_string(&Method::SysHello).unwrap() ==
  "\"sys.hello\""`. This is the property that actually matters for
  F1.3's "closed vocabulary, exact wire string" contract — a plain
  round-trip alone wouldn't catch a rename typo if both sides used the
  same (wrong) string consistently, so this is a REQUIRED addition
  beyond the generic round-trip pattern, not optional. Cover every
  variant in the enum (read the full file — this spec's grounding above
  only listed the first ~16 of what the file's `total_lines: 196`
  implies is a longer list; get the complete variant list yourself).
- R3. For `ContentBlock` (tagged enum in `session.rs`): assert the
  actual JSON shape includes the `"type"` tag key as documented
  (`#[serde(tag = "type")]`), not just that round-trip equality holds —
  e.g. deserializing `{"type": "text", "text": "hi"}` directly (not
  produced by this test's own serialization) must succeed, proving the
  wire CONTRACT (what an external/future client would send) works, not
  just that Rust-to-Rust round-tripping is self-consistent.
- R4. For any `Option<T>` field across all types: at least one test
  case per struct with the option set to `None` and JSON round-tripped
  — `serde` sometimes needs `#[serde(skip_serializing_if = "Option::
  is_none")]` to omit vs. emit `null`; read whether these structs have
  that attribute (they may not) and assert whatever the ACTUAL current
  behavior is, don't assume either shape.

### Non-Functional

- N1. Do NOT modify any file under `crates/pii-proto/src/` — test-only
  addition. If a genuine wire-format inconsistency is found (e.g. a
  `Method` variant whose rename string looks wrong), STOP and report it
  per the same discipline as spec-53's `MemStore::put` finding — do not
  silently "fix" production code in a test-coverage spec.
- N2. Standard test-file clippy allows apply
  (`unwrap_used`/`expect_used`/`panic`/`indexing_slicing`), matching
  every other integration test file's convention in this workspace.

## Architecture

```
crates/pii-proto/tests/
└── wire_roundtrip.rs   (NEW — method.rs/run.rs/session.rs/sys.rs types)
```

No production code changes.

## TDD Contract

| Test Name (illustrative — exact names are the implementer's choice, one per type/property) | Input | Expected Output |
|---|---|---|
| `method_*_roundtrips_and_matches_wire_string` | every `Method` variant | round-trips AND literal JSON string matches the documented `#[serde(rename)]` |
| `run_start_params_roundtrips` | `RunStartParams` with a non-trivial `input` JSON value and `caps: Some([...])` | equality holds |
| `run_list_params_roundtrips_with_none` | `RunListParams { status: None, since: None }` | equality holds; actual JSON shape asserted (null vs. omitted) |
| `wire_run_header_roundtrips` | `WireRunHeader` | equality holds |
| `session_create_params_roundtrips` | `SessionCreateParams` with `model: Some(...)`, `graph: None` | equality holds |
| `content_block_wire_shape_is_tagged` | hand-written `{"type": "text", "text": "..."}` JSON string | deserializes into `ContentBlock::Text` |
| `hello_result_roundtrips` | `HelloResult` with nested `PeerInfo`/`Limits` | equality holds |
| `status_result_roundtrips` | `StatusResult` with non-empty `workers`/`listeners`/`channels` vecs | equality holds |

## Exit Criteria

- [ ] `[ -f crates/pii-proto/tests/wire_roundtrip.rs ]`
- [ ] `cargo test -p pii-proto --test wire_roundtrip`
- [ ] `cargo test --workspace --lib --bins --tests`
- [ ] `cargo test --workspace --doc`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo fmt --all --check`
- [ ] `cargo run -q -p xtask -- silent-noop-check`
- [ ] `cargo run -q -p xtask -- lint-files`

## Guardrails

- G1. Do NOT modify any file under `crates/pii-proto/src/`.
- G2. Do NOT touch `crates/pii-proto/tests/` if any OTHER test file
  already exists there (there shouldn't be one — verify with `ls
  crates/pii-proto/tests/ 2>/dev/null` before starting; if one exists,
  STOP and report rather than guessing why the grounding above was
  wrong).
- G3. Do NOT touch `specs/` (read-only for the workhorse).
- G4. Do NOT commit — leave the diff in the working tree for the
  orchestrator to review and commit.
- G5. No `unsafe` code (`#![forbid(unsafe_code)]`, workspace-wide).
