# Spec: 97 — `pii-observe`: the observability contract every crate can depend on

**Project**: <repo>
**Phase**: P0/P1 (production readiness, `_docs/06-research/052-…md`) — observability first, per the
user's directive *"observability is a must"* and *"we must enforce observability! via traits on every
crate!"*. Design: `_docs/06-research/053-observability-contract-and-enforcement-2026-10-09.md`.
**This spec creates the CONTRACT ONLY.** Adoption, the enforcement lint and the health payload are
separate specs (98, 99, 100).

## Context
Today the trait lives in `pii-core` (`observe.rs`: `Observer` + `ObsEvent` + `Metric`), which is the
engine — 13 crates have zero observability and three (`pii-proto`, `pii-tui`, `pii-web`) cannot even
reach the trait. There are 139 raw `eprintln!`/`println!` sites in `src/`. The contract must move to a
crate that has no upward edges.

## Requirements
R1. **New workspace member `crates/pii-observe`.** Its dependencies are `serde` and `serde_json` ONLY.
    It must NOT depend on `pii-abi`, `pii-core`, `tokio`, or any other workspace crate — prove it with
    `cargo tree -p pii-observe` in the report (only serde/serde_json and their transitive deps may
    appear). Add it to the workspace `[members]` and to the root `[workspace.dependencies]` if the repo
    centralises deps.
R2. **The contract** (in `pii-observe`):
    - `Level` — `Trace`, `Debug`, `Info`, `Warn`, `Error` (serde, `Ord` so thresholds work).
    - `Event { subsystem: &'static str, level: Level, kind: &'static str, fields: Fields, span: Option<SpanId> }`
      with a `Fields` type that is cheap to build and serialises to a JSON object; and a `SpanId` newtype.
    - `Measure` — `Counter(u64)`, `Gauge(i64)`, `Histogram { count: u64, sum_us: u64, max_us: u64 }`.
    - `Observer` — `fn event(&self, e: &Event)`, `fn measure(&self, m: &Measure)`, `fn flush(&self)` (all
      with no-op defaults is acceptable ONLY if a `NullObserver` still exists as the canonical no-op).
    - `Observable` — the per-crate seam: `fn observer(&self) -> Option<&dyn Observer>`.
    - `Health` — `fn health(&self) -> SubsystemHealth` where `SubsystemHealth { name: String, state: State, detail: Option<String>, identity: BTreeMap<String, String> }` and `State { Ok, Degraded, Down }`
      (serde). `identity` is how a subsystem reports *what it actually is* (paths, versions, counts) —
      see spec 99 for the payload that consumes it.
    - `Observed`/`NoopObserver` helpers as needed for ergonomic no-op use.
R3. **A name registry** — `pub mod names` with `pub const` strings for the vocabulary already in use
    (`graph.node.dead`, `graph.join.unsatisfiable`, `graph.batch.width`, plus the event/measure names
    this contract introduces), so call sites and tests assert on symbols, not literals. A unit test must
    assert the registry has **no duplicates and no empty names** (scan the module's consts via a
    registration list you keep in the same module — not a `grep`).
R4. **`pii-core` re-exports it without breaking the 101 existing call sites.** `pii-core::observe`
    must keep `Observer`, `ObsEvent`, `Metric`, `NullObserver`, `RecordingObserver`, `MultiObserver`
    working (re-export, adapter, or thin shim — say which in the report). The existing graph metrics and
    the CLI ndjson observer must still compile and their tests still pass.
R5. **Module docs** in `pii-observe/src/lib.rs` stating: the contract's purpose, that `pii-observe` is
    the dependency floor (serde only, so any crate can depend on it), and that adoption + enforcement
    are specs 98/99/100.
R6. `specs/` is read-only for you; do NOT edit any spec file.

## Exit Criteria
- [ ] `cargo build --workspace` exits 0 and `cargo test -p pii-observe` exits 0
- [ ] `cargo tree -p pii-observe` lists ONLY serde/serde_json (+ their transitive deps) — no workspace crate
- [ ] `cargo test --workspace --lib --bins --tests` exits 0 (the re-export did not break the existing 101 call sites)
- [ ] `bash quality-gate.sh` exits 0 and `cargo run -q -p xtask -- fresh-gate --worktree` prints `fresh-gate: PASS`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` exits 0; `cargo fmt --all --check` exits 0
- [ ] a unit test proves the name registry has no duplicates/empties, and a serde round-trip test exists for `Event`, `Measure` and `SubsystemHealth`
- [ ] `lint-files` reports no file over the warn threshold (report its output)
- [ ] the report names every file added/changed and confirms `specs/` was not touched

## Guardrails
- CONTRACT ONLY: do NOT migrate call sites to the new types, do NOT add events anywhere, do NOT touch
  the gate script. The point of this spec is a stable, dependency-free contract.
- Do NOT put `tokio`, `parking_lot`, or any workspace crate into `pii-observe` — the dependency floor is
  the whole reason it exists.
- Do NOT delete the old types; they are the compatibility surface for 101 call sites.
- Do NOT weaken or delete an existing test.
- Scratch under `<scratch>/` only; delete your own exports (one fresh-gate export is ~8G).
- Do NOT commit; leave the tree for the orchestrator.

## Result + orchestrator verification (2026-10-09)
Landed: `crates/pii-observe/` (`lib.rs`, `event.rs`, `measure.rs`, `health.rs`, `observer.rs`, `names.rs`),
workspace member added, `pii-core` depends on it and re-exports it as `pii_core::contract` (the old
`Observer`/`ObsEvent`/`Metric`/sinks stay untouched, so the 101 call sites are unaffected). 10 turns, $0.33.

**Verified by the orchestrator, not taken on report:**
- **Dependency floor (the whole point)**: `cargo tree -p pii-observe` shows only `serde`/`serde_json` and
  their proc-macro transitives — **no workspace crate**.
- `cargo test -p pii-observe` → 9 passed, including `registry_has_no_duplicates_and_no_empties` and
  `existing_vocabulary_is_preserved`.
- **Mutation control 1 (the registry test has teeth)**: adding a second const with the same VALUE as
  `graph.node.dead` and registering it in `ALL` turns the test RED —
  `duplicate name graph.node.dead`. Restored, re-verified green.
- Both gates green: `quality-gate.sh` ALL PASS · `fresh-gate --worktree` PASS.

**FINDING (orchestrator, measured — spec 98 must close it):** the registry test only guards the `ALL`
slice, so a name const that is **never registered** is invisible. Proven: with
`pub const MUTANT_UNREG: &str = "graph.batch.width";` (a duplicate value, omitted from `ALL`) **all 9
tests still pass**. A new name can therefore be added and silently never enter the vocabulary the lint
and the dashboards read. Spec 98's `lint-observability` must enforce registry COMPLETENESS (scan
`names.rs` for `pub const` and require each to appear in `ALL`), not only uniqueness inside `ALL`.
