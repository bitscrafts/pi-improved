# pi-improved — spec-24: silent-noop-check

- **Project**: `<repo>`
- **Priority**: Highest-leverage work in the whole repo (opus review 011 Part B §24).
  It converts the silent-no-op class — declared-but-unwired surfaces — into
  machine-checkable facts, which is the strongest self-improvement machinery and
  the boss loop's fitness function. Build it FIRST; it would have caught four of
  opus's findings automatically.
- **Status**: SPECIFIED — not yet dispatched · *(lifecycle: SPECIFIED → IN PROGRESS
  on dispatch → IMPLEMENTED — awaiting review → DONE after the orchestrator
  commits and updates HANDOFF.md)*
- **Source**: `_docs/06-research/011-pii-architecture-conformance-review-pass1.md`
  Part B spec 24 (opus, 2026-09-09)
- **Depends-On**: none (new xtask subcommand; the tree is green at HEAD `47e551e`)

---

## Verified Premises

Each checked BEFORE this spec was written (2026-09-09, operator reading the tree):

- `crates/pii-proto/src/method.rs:29,38,59,62,65` — `Method::{SessionInterrupt,
  SessionClose, GraphApply, GraphGet, GraphList}` variants exist.
- `crates/pii-server/src/dispatch.rs` — those five have **zero** `Method::X =>`
  arms (grep counts 0 each); all fall to the catch-all `MethodNotFound` at
  `dispatch.rs:146`. (verified via `grep -c Method::$m dispatch.rs` = 0)
- `crates/pii-proto/src/event.rs:155,162,180` — `Event::{WorkerDied,
  WorkerRestarted, MemoryDegraded}` variants exist.
- Those three `Event` variants are constructed nowhere as broadcast events: grep
  finds them only in `crates/pii-server/src/subscribe.rs` (filter) and
  `crates/pii-tui` (render). `Event::AssistantDelta` (event.rs:76) IS produced
  (agent.rs, the spec-22 win) — proving the `Event` taxonomy is the observable
  contract.
- `crates/pii-server/src/daemon/config.rs:112,114` — `[memory] embedder`
  (`String`) and `cache_bytes` (`u64`) fields exist with defaults `trigram`/`0`
  (config.rs:121-122). Zero read sites outside declaration.
- `graphs/starter/assistant.toml:18` — `memory = { enabled = true, top_k = 6 }`.
  `top_k` has zero read sites in `crates/` Rust (grep of `top_k` hits only the
  two .toml graph files and a hardcoded `6` in agent.rs).
- `xtask/src/reachability.rs`, `xtask/src/doc_drift.rs` — the two existing
  no-model gates; the new `silent-noop-check` follows their shape
  (`run(workspace_root) -> Result<(), Vec<String>>`).
- `xtask/src/main.rs:63-68` — `run_ci` calls deps/net/lint/proto/bench/reachability/
  doc_drift/fmt/clippy/test; the new gate must be added beside reachability/doc-drift.

---

## Overview

The opus review found that this codebase's dominant pathology is **shaped but
not wired**: a declared surface with no consumer. Concretely at HEAD `47e551e`:

- 5 RPC methods on the wire (`session.interrupt`, `session.close`,
  `graph.apply`, `graph.get`, `graph.list`) answer `MethodNotFound`;
- 3 event variants (`WorkerDied`, `WorkerRestarted`, `MemoryDegraded`) have no
  producer;
- 2 config keys (`[memory] embedder`, `[memory] cache_bytes`) are read nowhere;
- `assistant.toml`'s `top_k` is read nowhere.

These survived because the gate set never checked "declared ⇒ used". This spec
builds **`cargo xtask silent-noop-check`**: a deterministic, no-model subcommand
that fails CI when a declared surface has no consumer, in the four classes above.
Wired into `run_ci`, it converts every future silent-no-op into a hard, comparable
regression — the same job `doc-drift-check` does for docs, done for the wire.

Crucially: on the current tree the gate must report **exactly these 11** and exit
1. The implementer does NOT fix the 11 (that is derived work for later specs /
direct fixes, e.g. spec 26 implements graph.apply/get/list). This spec ONLY builds
the gate + a green allowlist after each of the 11 is wired or allowlisted WITH a
written reason. The acceptlist-with-reason is the mechanism that keeps the gate
honest (see Requirements R1d).

---

## Requirements

### Functional

- **R1 (`silent-noop-check` subcommand)**: `cargo xtask silent-noop-check [--list]`
  exits 0 on a clean tree and 1 when any declared surface has no consumer or lacks
  an allowlist reason. Sub-checks:
  - **R1a (method arms)**: every `Method::X` variant in
    `crates/pii-proto/src/method.rs` must have a `Method::X =>` arm in
    `crates/pii-server/src/dispatch.rs`, OR be in an allowlist with a written reason.
    `--list` prints each method and its status (wired | allowlisted <reason> |
    MISSING).
  - **R1b (event producers)**: every `Event::X` variant in
    `crates/pii-proto/src/event.rs` must be constructed at a broadcast/producer
    site OUTSIDE `pii-proto`, `pii-server/src/subscribe.rs`, and `pii-tui`
    (filter + render sites are NOT producers), OR be allowlisted with a reason.
  - **R1c (config keys)**: every field of the `[memory]` section in
    `crates/pii-server/src/daemon/config.rs` (embedder, cache_bytes) must have a
    read site in `crates/`, OR be allowlisted with a reason. Same for every
    node-config key in `graphs/starter/*.toml` (e.g. `memory.top_k`).
  - **R1d (allowlist discipline)**: the allowlist lives in
    `xtask/src/silent_noop.rs` as typed entries `{ surface, class, reason }` where
    `reason` is a non-empty written justification. An allowlisted surface with an
    EMPTY reason is itself a violation (fails the gate). This keeps "we deferred
    it" as a deliberate, documented, reviewable act — not a hole.
- **R2 (wire into run_ci)**: `run_ci` in `xtask/src/main.rs` invokes
  `silent_noop::run()` beside reachability/doc-drift, so CI enforces it. `USAGE`
  lists `silent-noop-check`.
- **R3 (the 11 current findings)**: on the CURRENT tree, the gate reports exactly
  the 11 surfaces: the 5 methods, the 3 events, the 2 config keys, and `top_k`.
  The implementer resolves each: either wire it (create the consumer) or add a
  written allowlist reason. After resolution the gate exits 0. **No surface may
  be silently removed** (deleting a variant to dodge the gate is a violation).

### Non-Functional

- **N1**: the gate is deterministic and NO-MODEL — it reads source text and does
  not call a provider. A non-zero exit means a real miss, not a quota/blip.
- **N2**: no new runtime dependencies (xtask already has the std/serde it needs).
- **N3**: the quality gate stays green; the test count only goes up.
- **N4**: `silent-noop-check` must NOT decide for the human — it reports; it never
  fixes or rewrites files.

---

## Architecture

```
cargo xtask silent-noop-check
   ├─ scan_methods()   : method.rs variants  vs  dispatch.rs arms   (+ allowlist)
   ├─ scan_events()    : event.rs variants   vs  producer sites      (+ allowlist)
   ├─ scan_config()    : [memory] fields     vs  read sites          (+ allowlist)
   ├─ scan_graph_keys() : graphs/starter/*.toml node keys vs read sites (+ allowlist)
   └─ report()         : failures -> Err(vec![...]) -> exit 1
```

Follow `xtask/src/reachability.rs`'s shape exactly: `pub fn run(workspace_root:
&Path) -> Result<(), Vec<String>>`, one violation string per line prefixed
`SILENT-NOOP FAIL: ...` (matching how `net_check.rs` prefixes `NET-CHECK FAIL`).

**Key decisions.**

- **Source-text scanning, not rust-analyzer.** For methods we match `Method::X`
  as a declared variant vs `Method::X` appearing in a `Method::X =>` arm context;
  for events we match construction sites. A hand-written scanner is coarse but
  deterministic and adequate for this repo's enum shapes; a full semantic parser
  is out of scope and would itself be the thing to be wrong about. The
  synchronization: `method.rs` has the `snake_case` name strings (e.g.
  `"session.interrupt"`), so the scanner keys on the variant name.
- **Allowlist-with-reason beats deleting surfaces.** R3 mandates the implementer
  RESOLVE each of the 11. For the 5 methods: the opus review Part B lists
  `graph.apply/get/list` as needed by the boss (spec 26/27) — those may be
  implement-if-trivial OR allowlisted with reason `"wired by spec 26/27"`; but
  `session.interrupt`/`session.close` (A in review: "should be implemented, since
  run.cancel and session lifecycle already exist") must be WIRED now because their
  dispatch arms are small and load-bearing. The events/config keys/top_k are
  allowlisted WITH reasons (their consumers land in later specs) unless the
  implementer chooses to wire them. The point of R3 is that each is a deliberate,
  documented decision — not that everything be wired in this one spec.
- **`--list` for the operator** — the human-readable inventory (`path:line` +
  status + allowlist reason) that makes the gate's decisions auditable at a
  glance.

**What this spec is not**: it does NOT implement graph.apply/get/list (spec 26),
does NOT wire event producers (later), does NOT read `top_k`/config key semantics
(later). It builds the detector and forces an explicit, documented resolution of
the 11 current instances.

---

## TDD Contract

| id | test | given | expects |
|----|------|-------|---------|
| R1a | `method_without_arm_fails` | a fabricated method variant with no dispatch arm and no allowlist | `scan_methods` returns that method as a violation |
| R1a | `allowlisted_method_pass` | a method variant that IS allowlisted with a reason | NOT a violation |
| R1a | `allowlisted_method_empty_reason_fails` | a method in the allowlist with an empty reason | violation (R1d) |
| R1b | `event_without_producer_fails` | an event variant with no producer site outside filters/tui | violation |
| R1c | `config_key_without_reader_fails` | a config field with no read site | violation |
| R1d | `wired_surface_ok` | a method/event/key that genuinely has a consumer and no allowlist | OK (no violation) |
| R2 | `run_ci_includes_silent_noop` | `run_ci` source body | it calls `silent_noop::run` and USAGE lists `silent-noop-check` |
| R3 | `real_tree_reports_11` | the actual method.rs/event.rs/config.rs/assistant.toml at HEAD | `scan_*` collectively flag >= 10 of the 11 known surfaces (they may be allowlisted; the point is the scanner finds them) |

**R3 is the acceptance test.** A plausible implementation passes R1a-R1d against
fixtures but ships a scanner that misses the real enum shapes (e.g. keys off the
snake_case string instead of the variant, or a producer regex that matches the
subscribe.rs filter). The test must read the REAL files and assert the 11 known
surfaces are all detectable. It does NOT require the allowlist to be empty after
resolution — it requires the scanner to FIND them.

---

## Exit Criteria

- [ ] `cargo build -p xtask --all-targets` — compiles (R1,R2)
- [ ] `cargo test -p xtask silent_noop` — the 8 TDD-contract tests pass, non-vacuously (`grep -qE "^test result: ok\\. [1-9]"`) (R1,R3)
- [ ] `cargo test -p xtask run_ci_includes_silent_noop` — the R2 test passes (R2)
- [ ] `timeout 60 cargo xtask silent-noop-check` — exits 0 (R3 resolved: all 11 wired or allowlisted with reasons)
- [ ] `timeout 60 cargo xtask silent-noop-check --list` — prints each of the 11 surfaces with status/allowlist reason; no surface is MISSING (R3)
- [ ] `cargo xtask silent-noop-check` — the gate itself passes via run_ci too
- [ ] `cargo test --workspace` — 0 failures; test count >= baseline (N3)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` — 0 errors (N3)
- [ ] `cargo fmt --all --check` — clean (N3)
- [ ] `cargo xtask deps-check` + `cargo xtask net-check` — pass (N3)

**Prose criteria**:

1. Test counts pasted raw, one line per binary, **unsummed**.
2. The `--list` output must name all 11 surfaces with a non-empty reason each —
   the empty-reason guard (R1d) must also be demonstrated (a fabricated empty
   allowlist entry fails the gate).

---

## Guardrails

- **G1 — do NOT edit this spec.** If it is wrong, STOP and report it.
- **G2 — do NOT commit.** Leave work in the working tree.
- **G3 — do NOT weaken, skip or delete an existing test.**
- **G4 — do NOT delete a `Method` or `Event` variant to dodge the gate** (R3/N4) — surfaces are wired or allowlisted, never silently removed from the enum.
- **G5 — no hardcoded absolute paths.** Test artefacts under `_tmp/` or the workspace.
- **G6 — report raw output, never summed totals.**
- **G7 — allowlist reasons must be written, non-empty, and specific** ("wired by spec 26" > "deferred").

### Error handling expectations

- A surface that "could not be determined" (e.g. a method variant whose snake_case
  name string is missing) must be treated as PRESENT and thus UNWIRED → violation.
  The scanner must not silently skip an unparseable variant — that is the exact
  shadow gap it exists to close.
- `empty reason` on an allowlisted surface is a violation (R1d).
- `--list` is the operator repo; the exit code is the machine contract.

---

## Files to Modify

| File | Change |
|------|--------|
| `xtask/src/silent_noop.rs` | new: `run`, `scan_methods`, `scan_events`, `scan_config`, `scan_graph_keys`, allowlist (R1) |
| `xtask/src/main.rs` | add `mod silent_noop;` + `Some("silent-noop-check")` arm + wire `report(silent_noop::run(&workspace_root()))` into `run_ci` (R2) |
| `xtask/src/main.rs` | `USAGE` const: list `silent-noop-check` (R2) |
| `xtask/src/main.rs` | tests: `run_ci_includes_silent_noop` (R2) |
| `xtask/src/silent_noop.rs` | tests: R1a-R1d + R3 real-tree acceptance |

**Not modified**: `crates/*` source (the 11 surfaces are resolved via the
allowlist, not by changing the engine in this spec). `dispatch.rs`, `event.rs`,
`config.rs`, `assistant.toml` stay byte-identical unless a method is wired — and
`session.interrupt`/`session.close` if wired, only by adding their dispatch arms
in the existing style.