# pii — ground truth for fable review (2026-09-18)

Pre-researched facts so fable can skip discovery and go straight to
analysis. Read this file first; do not re-derive what's already here
(saves budget). Everything below was measured directly against the
real repo tree today, not recalled from memory.

## Project identity

- Root: `/projects/pi-improved`. Rust workspace, 20 crates (`pii-*`).
- `AGENTS.md` (repo root) is the authoritative workflow doc — read it
  for the pidag SDD workflow, cost policy, and hard rules.
- Backlog status: specs 43 through 61 are ALL implemented, gate-
  verified, and committed. `HANDOFF.md`'s current-state header
  reflects this (commit `35c4353`). There is no pending mechanical
  work — everything left is a design/architecture decision (see
  "Open decisions" below).

## Crate map (name -> approx src LOC -> pii-* dependencies)

```
pii-abi        1343   (none — base types/errors, everyone depends on this)
pii-core       2214   pii-abi
pii-proto      1456   (none — wire types)
pii-uds         120   pii-abi, pii-core
pii-net        2359   pii-abi, pii-core
pii-store      3129   pii-abi, pii-core
pii-model      1960   pii-abi, pii-core, pii-net
pii-channels    482   pii-abi, pii-core
pii-memory     1140   pii-abi, pii-core, pii-store
pii-graph      4968   pii-abi, pii-core, pii-store
pii-http-fetch  358   pii-abi, pii-core, pii-model, pii-net
pii-tools      1820   pii-abi, pii-core, pii-model, pii-proto, pii-uds
pii-worker      919   pii-abi, pii-core
pii-evolve      358   pii-core, pii-proto
pii-tui         739   pii-proto
pii-web         380   pii-abi, pii-proto, pii-uds
pii-mcp         357   pii-abi, pii-core, pii-tools, pii-proto, pii-server
pii-server     7821   pii-abi, pii-core, pii-store, pii-graph, pii-proto,
                       pii-model, pii-memory, pii-tools, pii-net,
                       pii-worker, pii-channels
pii-cli        2529   pii-abi, pii-core, pii-store, pii-graph, pii-proto,
                       pii-server, pii-uds, pii-worker, pii-tools, pii-tui
```

`pii-server` is the hub (11 internal deps, 7821 LOC — the biggest
crate by a wide margin) and `pii-cli` is the outermost consumer (10
internal deps). `pii-abi`/`pii-core` are the only crates every other
crate transitively depends on.

## 450-line file policy (measured today, current offenders)

Standing policy: files should stay under ~450 lines (splits happened
tonight for several files that crossed it — see commits `aaee53c`,
`39db2e1`, `b98a581`, `11df84d`, `629ac9f`, `8988f5f`, `a337ab2` for
the pattern used: split into `module/{mod,submod1,submod2}.rs`, keep
the public API surface identical). CORRECTION (spec 69): the list below
was originally measured with raw `wc -l`, which is NOT the enforced
metric — the gate (`cargo xtask lint-files`) counts effective lines,
excluding comments, blanks, and `#[cfg(test)]` bodies. Re-measured with
`cargo run -q -p xtask -- lint-files --report` (effective, raw, path):
NO file is currently over the 450 effective-line gate (max 360); the
"13 offenders" claimed earlier were an artifact of the wrong metric.
Top of the current list, for reference:

```
360  627  crates/pii-store/src/lib.rs
358  403  crates/pii-server/src/engine/dispatch.rs
350  444  crates/pii-server/src/daemon/mod.rs
347  458  crates/pii-graph/src/sched/dispatch.rs
341  801  crates/pii-model/src/ollama.rs
335  481  crates/pii-graph/src/validate.rs
329  511  crates/pii-server/src/lib.rs
306  385  crates/pii-net/src/listen.rs
303  557  xtask/src/deps.rs
300  566  crates/pii-store/src/session.rs
```

These are candidates for the SAME split pattern already used
successfully tonight — do not propose a different mechanism without a
reason; consistency with the existing pattern reduces review cost.

## Test coverage snapshot (measured today)

41 integration test files under `crates/*/tests/`. Every crate has
SOME direct coverage now (F-20 closed tonight). Thin-but-nonzero
areas noted in prior sessions: `pii-web` (2 tests), some `pii-cli`
command files still untested at the integration level (only
`cli_util.rs`/`ndjson.rs` got direct coverage tonight, per spec-58 --
`cmd_ctl.rs`/`cmd_daemon.rs`/`cmd_run.rs`/`cmd_sandbox_exec.rs`/
`cmd_soak.rs`/`cmd_sys.rs`/`cmd_tui.rs`/`cmd_worker_exec.rs` remain
without dedicated test files, though they may have inline
`#[cfg(test)]` coverage — verify before assuming a gap).

## Known, deliberately-deferred items (do not re-propose these as new findings)

- **F-11** (string-typed `PiiError::Internal` at ~817 call sites): a
  bounded pilot landed (`crates/pii-store/src/heal.rs`, commit
  `c8fb4d9`, a `StoreOp` structured error enum replacing ad-hoc
  strings in that one file). The full sweep across ~30 files is
  DELIBERATELY out of scope for one spec — it needs to be split into
  several bounded passes, one crate/module at a time, following the
  `heal.rs` pilot's pattern. If you think F-11 needs more work, the
  finding is "expand the StoreOp pattern to N more files," not
  "PiiError needs restructuring" (the enum itself, in
  `crates/pii-abi/src/error.rs`, is already well-designed — 8
  variants, `is_retryable()`/`is_terminal()` methods, tested).
- **Coding-harness graph** (implement -> validate -> gate -> iterate
  as a real `pii` graph, self-hosting spec-driven development inside
  pii itself): design-only, no code. Full reasoning in
  `_docs/06-research/029-coding-harness-design-questions-for-opus.md`
  and the compact entry `_docs/06-research/030-OPUS-START-HERE.md`.
  Grounded against 3 real starter graphs already in the repo:
  `graphs/starter/verify-fix.toml` (closest precedent),
  `graphs/starter/heal-fix.toml`, `graphs/starter/failover.toml`.
- **Modularity idea** ("microservices in a single binary" — memory/
  MQTT/harness/Chromecast as swappable modules via the graph engine's
  uniform node interface): idea-stage, in the same 029 doc.
- **DAG format unification with pidag** (`/projects/pidag`, a sibling
  project): USER REQUIREMENT, two-phase — compatible principles now
  (already true: both are declarative-only, no runtime-code routing),
  identical shared schema later, once a concrete driver exists (e.g.
  the coding-harness graph, if it gets built). Direction recorded as
  a recommendation (pidag becomes a constrained SUBSET of pii's
  port-based schema, not the reverse) pending opus confirmation. Full
  reasoning in `_docs/06-research/030-OPUS-START-HERE.md`'s
  "USER-MANDATED" section. `/projects/pidag/docs/
  pii-vs-pidag-dag-format-notes.md` has the detailed format
  comparison if useful context (a prior fable review of pidag wrote
  this — same reviewer, different project, worth reading for
  consistency of analysis style).
- **`_docs/06-research/` reorganization** (28+ files, flat-by-number,
  no thematic grouping): hygiene question, low stakes, queued for
  opus alongside the above.

## What a NEW finding should look like

Given the above is already known, a genuinely new/useful finding from
this review should be one of:
1. A concrete bug or correctness issue not already tracked (the
   pidag review found 2 real ones this way: a quality-gate script
   misconfiguration, and a test allowlist regression — both were
   surfaced by actually running things, not just reading code).
2. A specific, scoped spec proposal for one of the deferred items
   above (e.g. "F-11 pass 2: apply the StoreOp pattern to
   crates/pii-server/src/engine/runs.rs's N call sites" — concrete
   enough to hand to a pidag worker directly).
3. A cross-cutting pattern across the 13 over-450-line files that
   suggests a SHARED root cause worth fixing once (e.g. if 5 of them
   are oversized because of the same kind of test-fixture bloat,
   that's a more valuable finding than 5 separate split proposals).

Avoid re-deriving the crate map, the dependency graph, or the
test-coverage state — all measured and included above.
