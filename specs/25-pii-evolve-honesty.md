# pi-improved — spec-25: pii-evolve-honesty

- **Project**: `/projects/pi-improved`
- **Priority**: Make the pii-evolve crate's documented claims TRUE before it
  grows. The opus review (011) found it is the first module whose doc comments
  assert properties the code does not have (A14/A15/A16). A doc surface that lies
  is worse than a missing feature — it is what the gate builders read.
- **Status**: SPECIFIED — not yet dispatched · *(lifecycle: SPECIFIED → IN PROGRESS
  on dispatch → IMPLEMENTED — awaiting review → DONE after the orchestrator
  commits and updates HANDOFF.md)*
- **Source**: `_docs/06-research/011-pii-architecture-conformance-review-pass1.md`
  Part B spec 25 (opus, 2026-09-09) — fixes A14, A16.
- **Depends-On**: spec 24 (the silent-noop gate gives the negative-check harness
  this spec's N4 relies on); tree green at HEAD `47e551e`.

---

## Verified Premises

Each checked before writing (2026-09-09):

- `Cargo.toml:20` — `"crates/pii-evolve"` is an **unconditional workspace member**.
- `grep '[features]'` across all manifests — only `pii-net`, `pii-proto`,
  `pii-server` (sole feature `testkit`). **There is no `evolve` Cargo feature.**
- `crates/pii-evolve/Cargo.toml` — depends on `pii-core` only (no pii-graph,
  pii-store, pii-server).
- `grep -l "pii-evolve" crates/pii-cli/Cargo.toml crates/pii-server/Cargo.toml`
  → empty. No crate depends on it; the default `pii` binary does NOT link it
  (verified in spec 23: no evolve symbols in `target/release/pii`).
- `crates/pii-evolve/src/lib.rs:4-8` — doc comment says: *"A dedicated,
  feature-gated crate: it is not a dependency of pii-cli or pii-server, so the
  default workspace build links none of it — an installer that does not want
  self-improvement builds no boss code and spends no boss tokens."*
- `grep -rn "RoutePolicy|CompactionPolicy|summarize_for_compaction|RouteLog"
  crates/` including every caller except `pii-evolve/src/{routing,context}.rs`
  and `tests/{routing,context}_tests.rs` → EMPTY. `routing.rs` and `context.rs`
  are dead (only test callers).
- `grep -rn "\.summarize()" crates/` except `state_tests.rs` → empty.
  `StateDoc::summarize` has only a test caller.
- `crates/pii-evolve/src/driver.rs:144` — `Ctx::root(caps, Budget::unlimited())`.
  driver imports `Budget` at driver.rs:14 (for the type) but passes unlimited.
- `crates/pii-evolve/src/main.rs:37-51` — `--policy FILE` flag reads a file whose
  contents are a single directory path.

---

## Overview

Spec 23 shipped the pii-evolve crate as a standalone crate with its own binary
(`pii-evolve`), matching the actual repo pattern for `pii-web`/`pii-mcp` (standalone
crates, own bins, NOT pulled into the `pii` binary). That much is fine and genuine:
the default build genuinely does not link or run boss code.

Three claims in the crate's docs, however, are false, and a doc surface that lies
is what every gate in this repo reads:

1. **"feature-gated"** (lib.rs:4). There is no Cargo feature named `evolve`. The
   crate is an unconditional workspace member. The *true* property is
   "standalone crate, not a dependency of pii-cli/pii-server, so not linked into
   the pii binary." That is real — but it is NOT "a Cargo feature." The word
   "feature-gated" must become true (add the feature) or the doc must say what is
   true (standalone opt-in crate, matching pii-web/pii-mcp).
2. **"runs on the substrate it heals"** (implied throughout, and 008 §3/§4 is
   explicit). The crate depends on `pii-core` alone. That is a *scoping* decision,
   not yet a lie, but combined with (3) below it is.
3. **`routing.rs` and `context.rs` are dead** — shaped, tested, inert. "Spec 23
   shipped the parts without the wiring" (opus A16).

This spec makes the crate's docs say exactly what the code does, and removes the
dead-shaped-but-not-wired modules rather than shipping a third pass of them.

**The decision** (matching how pii-web/pii-mcp are actually delivered in this repo):
make the doc comment true by describing the real mechanism — **standalone opt-in
crate, not linked into the `pii` binary, built only when an operator chooses to**
— and add a real `evolve` feature so "opt-in" is also mechanically true at the
Cargo level. `routing.rs` + `context.rs` are deleted (their logic re-enters when
they get a real consumer in spec 27/28); `driver.rs` takes a real budget.

---

## Requirements

### Functional

- **R1 (make the docs true)**: `crates/pii-evolve/src/lib.rs` (and crate
  `description` in Cargo.toml) must state the TRUE delivery — **standalone opt-in
  crate, not a dependency of `pii-cli`/`pii-server`, so the default `pii` binary
  neither links nor runs it** — matching how `pii-web`/`pii-mcp` are actually
  delivered (separate crate + own binary, not a subcommand of `pii`). The doc must
  NOT claim "a Cargo feature named `evolve`" unless such a feature exists in
  `cargo metadata`.
- **R2 (no dead modules)**: delete `crates/pii-evolve/src/routing.rs` and
  `crates/pii-evolve/src/context.rs` (and `tests/routing_tests.rs`,
  `tests/context_tests.rs`). `state.rs` (`StateDoc` incl. `summarize`) and
  `driver.rs` remain. `spec 24`'s `silent-noop-check`, extended to flag `pub` items
  in `pii-evolve` with only test callers, reports zero.
- **R3 (real budget)**: `driver::run_cycle` takes a real `pii_core::Budget` (not
  `Budget::unlimited()` internally) and returns `CycleOutcome::BudgetExhausted`
  when the gate-call budget is consumed. The `gate_ctx` helper builds the PROC
  `Ctx` with that budget.
- **R4 (working skeleton retained)**: the crate keeps `state.rs` (with
  `summarize`), `driver.rs`, `main.rs`, `driver_tests.rs`, `state_tests.rs`. The
  `pii-evolve <state-dir>` binary still runs a cycle against the deterministic
  gates.

### Non-Functional

- **N1**: the default `pii` binary must be byte-identical to HEAD before this spec
  (nothing a default build links changes). Verify via `blake3` on
  `target/release/pii` before vs after.
- **N2**: no new third-party dependencies.
- **N3**: the quality gate stays green; the test count may go UP, and the deleted
  dead-module tests leave (their code is removed; specs 27/28 re-add the
  capability with a real consumer).
- **N4**: no doc may claim a Cargo feature that `cargo metadata` does not list.
- **N5**: `cargo test --workspace` still passes; retained state/driver tests pass.

---

## Architecture

```
cargo build                     -> no pii-evolve in the graph (R1)
cargo build --features evolve   -> pii-evolve compiled, its bin produced (R1)
pii-evolve/src/lib.rs           -> doc comment = TRUE statement (R2)
crates/pii-evolve/src/          -> state.rs, driver.rs, main.rs (R3)
                                    [routing.rs, context.rs DELETED]
driver::run_cycle(state_dir, budget, policy) -> real Budget, BudgetExhausted (R4)
```

**Key decision — the honest mechanism is "standalone opt-in crate with a real
`evolve` feature", matching pii-web/pii-mcp's delivery.** The opus review asked
for "feature-gated, default off." pii-web/pii-mcp do this as standalone crates
that the default `pii` binary doesn't depend on — correctly not-linked, but with
no Cargo feature. Spec 25 closes the gap: add the real `evolve` feature so "opt-in"
is mechanical, and fix the doc to describe that truth. We do NOT attempt to wire
`pii-evolve` into `pii-cli`'s subcommand tree (that's a later consumer decision);
the crate stays an independent binary.

**Key decision — delete the two dead modules.** 008 §3.1/§3.2 describe routing and
context as capabilities the boss must have. They deserve a real consumer, not a
shelf. Spec 27 (graph loop) and spec 28 (model routing) are where they get wired;
shipping them inert until then is precisely the silent-no-op class. Deleting now
and re-adding with a caller later is cheaper than maintaining two tests-only modules.

**What this spec is not**: it does NOT build the boss loop (spec 27), does NOT wire
model routing (spec 28). It makes spec-23's crate honest and lean.

---

## TDD Contract

| id | test | given | expects |
|----|------|-------|---------|
| R1 | `lib_rs_doc_matches_delivery` | read `crates/pii-evolve/src/lib.rs` | states standalone-opt-in (not a dep of pii-cli/server); does NOT claim a Cargo `evolve` feature |
| R1 | `default_build_unchanged` | `blake3` of `target/release/pii` before vs after default build | hashes equal (N1) |
| R2 | `routing_context_deleted` | `test ! -e crates/pii-evolve/src/routing.rs && test ! -e crates/pii-evolve/src/context.rs` | both files absent |
| R3 | `run_cycle_zero_budget_exhausts` | call `run_cycle` with a zero-gate-call `Budget` | returns `CycleOutcome::BudgetExhausted` |
| R4 | `state_doc_round_trips` (retained) | write StateDoc to tempdir, re-read | fields equal, monotonic updated_at |
| R4 | `driver_green_cycle_ok` (retained) | run_cycle with green fixture + sufficient budget | `CycleOutcome::Ok`, state persisted |

---

## Exit Criteria

Machine-checked (each runs >=1 test; non-vacuity via `grep -qE "ok\\. [1-9]"`):

- [ ] `cargo build --workspace` — pii-evolve still builds as a standalone member; the default `pii` binary links none of it (R1, N1)
- [ ] `test ! -e crates/pii-evolve/src/routing.rs && test ! -e crates/pii-evolve/src/context.rs` — dead modules gone (R2)
- [ ] `cargo test -p pii-evolve` — retained state/driver tests pass, non-vacuously (R3, R4)
- [ ] `cargo test -p pii-evolve run_cycle_zero_budget_exhausts` — the R3 acceptance test passes (R3)
- [ ] `cargo test --workspace` — 0 failures (N3/N5)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` — 0 errors (N3)
- [ ] `cargo fmt --all --check` — clean (N3)
- [ ] `cargo xtask deps-check` + `cargo xtask net-check` + `cargo xtask silent-noop-check` — pass (N4)
- [ ] `cargo metadata` shows no `evolve` feature; `doc-drift-check` does not flag `pii-evolve`'s lib.rs (R1/N4)
- [ ] `git diff --quiet -- crates/pii-web crates/pii-mcp crates/pii-cli crates/pii-server` — those crates untouched (N1-negative)

**Prose criteria**:

1. Test counts pasted raw, one line per binary, **unsummed**.
2. The `lib.rs` doc comment before/after shown, proving R1.

---

## Guardrails

- **G1 — do NOT edit this spec.** If it is wrong, STOP and report it.
- **G2 — do NOT commit.** Leave work in the working tree.
- **G3 — do NOT weaken, skip or delete an existing test** — except the two
  dead-module test files (routing_tests.rs, context_tests.rs) whose code is
  deleted with their modules (R3), and ONLY if the crate name docs say they are
  gone. Retained state/driver tests must stay intact.
- **G4 — do NOT touch `crates/pii-web`, `crates/pii-mcp`, `crates/pii-cli`,
  `crates/pii-server`, or the `pii` binary's build graph** (N1). The `evolve`
  feature must leave the default `pii` byte-identical.
- **G5 — no hardcoded absolute paths.** Test artefacts under `_tmp/` or the workspace.
- **G6 — report raw output, never summed totals.**
- **G7 — the deleted modules must be re-added (spec 27/28) WITH a caller, never
  re-shipped inert.** Note this in the commit/tree as the reason for the delete.

### Error handling expectations

- If a faithful "optional member / build-exclusion" in Cargo is NOT achievable,
  use the documented fallback (remove from members + optional path dep) and
  REPORT it — do not silently ship a broken `evolve` feature. The observable
  contract (absent-by-default, present-on-feature) is the measure of success.
- `run_cycle` must fail LOUDLY (BudgetExhausted) on a zero budget, not swallow it.

---

## Files to Modify

| File | Change |
|------|--------|
| `Cargo.toml` | add `evolve` feature; make `pii-evolve` optional (member exclusion or optional path dep) (R1) |
| `crates/pii-evolve/Cargo.toml` | `description` stating standalone opt-in truth (R2) |
| `crates/pii-evolve/src/lib.rs` | doc comment = TRUE statement (R2); remove `pub mod routing; pub mod context;` (R3) |
| `crates/pii-evolve/src/driver.rs` | `run_cycle` takes real `Budget` + `BudgetExhausted` outcome; `gate_ctx` uses it (R4) |
| `crates/pii-evolve/src/main.rs` | pass a real budget (from a flag or a sane default) (R4) |
| `crates/pii-evolve/src/routing.rs`, `context.rs` | DELETED (R3) |
| `crates/pii-evolve/tests/routing_tests.rs`, `context_tests.rs` | DELETED with their modules (R3) |

**Not modified**: `pii-web`, `pii-mcp`, `pii-cli`, `pii-server`, any crate the
default `pii` binary links. The `evolve` feature is additive and opt-in.