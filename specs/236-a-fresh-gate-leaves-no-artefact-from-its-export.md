# Spec: 236 — a fresh-gate leaves no artefact from its export behind

**Project**: <repo>
**Phase**: filed 2026-10-23T00:35Z by the autonomy orchestrator, from a red measured during a tick's own
re-verification of HEAD `16236dd`. No worker was dispatched and nothing was spent.

## Overview

`fresh-gate` exports HEAD with `git archive` to `<scratch>/fresh-gate-<pid>/`, runs `quality-gate.sh`
inside that export, and **removes the export on success**. Its compiler output does not have to be discarded
with it — on purpose: `FRESH_GATE_TARGET` reuses one warm cache so the tree stays fresh while the build stays
incremental (40-60 minutes of cold build per attempt was the trap that introduced the export into the loop).

The warm cache is shared with the **routine** gate (`_tmp/gate-target`), and that is the defect. A Rust test
binary bakes `env!("CARGO_MANIFEST_DIR")` **at compile time**. A binary compiled inside an export therefore
carries `<scratch>/fresh-gate-<pid>/crates/...` — a path that stops existing the moment the run succeeds.
Cargo's fingerprint does not know that: the next run finds the artefact "fresh" and reuses it.

Measured on 2026-10-23, same tree, same HEAD, twice:

- **Run 1** `cargo run -q -p xtask -- fresh-gate` (00:16Z) — export `<scratch>/fresh-gate-73534`,
  `FRESH_GATE_TARGET` unset, so the gate's test step wrote to `_tmp/gate-target/test-gate`.
  **PASS**, `fresh_gate_rc=0`. The export was then removed — as designed.
- **Run 2** (00:24Z) — `test (lib/bins/tests)` **FAILED** on three `pii-evolve` tests
  (`driver_spawns_no_subprocess`, `launcher_dials_through_the_uds_client`,
  `no_raw_dial_remains_in_pii_evolve`) with
  `read driver.rs: Os { code: 2, kind: NotFound }` and `read src: Os { code: 2, ... }`.
  The suite read `1 passed; 3 failed` — **five** tests were collected, so nothing was deleted.
- The binary behind it: `_tmp/gate-target/test-gate/debug/deps/launcher-02e24e7123aaff6a`, mtime
  `2026-10-23T00:19:42Z` (i.e. compiled by run 1), `strings` → `<scratch>/fresh-gate-73534/crates/pii-evolve`,
  which no longer existed. A freshly compiled binary for the same tests in the same directory
  (`launcher-5ea7fb6082780f01`, 00:29:00Z) carries `crates/pii-evolve` and passes 5/5.

This is worse than a slow gate. A gate that fails on a clean tree at an unchanged HEAD teaches everyone to
ignore it, and the same mechanism can go the other way: an artefact compiled in the **repo tree** reused by an
export means the "CI-equivalent" run never compiled what it certified. `crates/` has 188 files reading
`env!("CARGO_MANIFEST_DIR")` and 190 call sites passing it to `pii_store::testing::tmp_scratch`, so the stale
binary is not only a reader — with `create_dir_all` it can **recreate a deleted export's directories** on the
next run. One class, many sites; the fix belongs in the gate that hands the cache out, not in 188 tests.

`quality-gate.sh --fresh` already knows a piece of this (it pins `FRESH_GATE_TARGET` to
`_tmp/gate-target-fresh` so the export does not write into the routine cache, measured 2026-10-22). It is not
enough: that directory is shared between runs, so run 2 of *any* fresh-gate still reuses run 1's export-baked
binary. And the plain `cargo run -p xtask -- fresh-gate` door — the one HANDOFF quotes — bypasses the pin
entirely and writes straight into the routine cache.

## Requirements

### Functional

- **R1 — the test step's artefacts die with the run that made them.** `fresh-gate` hands the gate child a
  **per-run** target directory for the `test (lib/bins/tests)` step, inside the export it is about to remove
  (a new `TEST_GATE_TARGET`, e.g. `<export>/_tmp/test-gate`). `gate-steps.txt`'s test line reads it with the
  old value as fallback: `CARGO_TARGET_DIR="${TEST_GATE_TARGET:-${FRESH_GATE_TARGET:-_tmp/gate-target}/test-gate}"`.
  A run that fails keeps its export, so its test artefacts stay inspectable with it; a run that succeeds
  removes both together.
- **R2 — the warm cache still carries everything else.** `fmt`, `clippy`, `clippy (hal-gpiod)`, `build`,
  `test (doc)` and the xtask scans keep resolving through `FRESH_GATE_TARGET` exactly as today. The change
  moves ONE step's cache. A fresh-gate must not become a cold workspace build (the measured 40-60 min trap).
- **R3 — no shared TEST-STEP cache holds an export path after a run.** After `fresh-gate` returns, no file
  under a shared `_tmp/gate-target*/test-gate` directory names a `<scratch>/fresh-gate-` path. Checked by
  grepping the binaries, not by trusting the code. **The scope is the test step on purpose, and it was measured
  on 2026-10-23**: `grep -rl` over `_tmp/gate-target` found **13437** files naming an export path, of which
  **3589** were under `test-gate` and the rest under `clippy-gate`/`build-gate` — steps R2 keeps on the warm
  cache, whose rmeta is never executed and whose export paths are the price of the 40-60 minute cold build R2
  rules out. A criterion written over *every* cache contradicts R2 and can only be met by the
  clean-after-every-run step the non-goals forbid; the defect under repair is a stale **test binary**, so the
  check owns `*/test-gate` and reports the out-of-scope count rather than hiding it.
  **The pattern is a baked manifest dir, not the bare prefix, and that was measured too.** The first version of
  the check greped for `<scratch>/fresh-gate-` and flagged **three** files in a correct tree: the `xtask`
  test binary and its incremental artefacts, whose string table carries the *sample* path
  `<scratch>/fresh-gate-7` that the new unit test uses as data (next to
  `_tmp/gate-target`, the warm dir). A check that fires on a correct tree teaches
  everyone to ignore it, so the pattern is `<scratch>/fresh-gate-[0-9]+/(crates|xtask)/` — the manifest
  dir a binary bakes at compile time, which is exactly the path the three launcher tests then fail to read.
  **The check carries two controls on every run:** it must see a planted baked manifest dir, and it must NOT
  flag the unit test's bare sample path.
- **R3a — the criterion names the target that holds the test.** `xtask` is a binary crate whose integration
  tests live in `--bins`, so `cargo test -p xtask --lib gate_env` collects **0 tests and exits 0** — measured
  2026-10-23 (`running 0 tests … 33 filtered out`, rc 0). Every criterion for this test therefore runs
  `--bins` and asserts `test result: ok. [1-9]`, never just rc 0.
- **R4 — the routine gate is unaffected.** The pii-evolve launcher lane on the routine cache
  (`CARGO_TARGET_DIR=_tmp/gate-target/test-gate`) reads 5 passed / 0 failed immediately after a fresh-gate run.
- **R5 — two consecutive runs both pass.** The measured red is the criterion: run `fresh-gate` twice in a row
  on a clean tree at one HEAD; both runs must report `fresh-gate: PASS`. The property in R3 is necessary and
  not sufficient, so this black-box pair stays in the exit criteria.
- **R6 — mutation control.** Removing the `TEST_GATE_TARGET` assignment (so the step falls back to the shared
  cache) must redden the new property test; the file is restored byte-identical (`sha256sum` before/after,
  `touch`, then green again) and both readings are pasted into `## Closure`.

### Non-goals, stated so a STOP is only for a genuine mismatch

- **Not** a change to the three `pii-evolve` launcher tests. They read their own source on purpose (spec 130
  R1/`no_raw_dial_remains_in_pii_evolve`); they were right and the binary was stale. Weakening them is a
  breach, not a fix.
- **Not** a change to `pii_store::testing::tmp_scratch` or to the 188 `CARGO_MANIFEST_DIR` sites.
- **Not** a wider "clean the cache after every run" step: deleting a shared warm cache is what makes the gate
  cost an hour, and it is not needed once the test step stops writing into it.
- **Not** the export's own removal policy on success (existing behaviour, unchanged).

### The shape of the change (so the worker does not have to invent it)

`xtask/src/fresh_gate.rs`'s `run_gate` builds the gate child's environment (`CARGO_TARGET_DIR`,
`QG_SKIP_COVERAGE`). The env is what the test should assert on: extract it into a small function returning
the pairs (e.g. `fn gate_env(export: &Path, warm: &Path, skip_coverage: &str) -> Vec<(&'static str, OsString)>`)
and have the test assert that `TEST_GATE_TARGET` is present, lies under the export, and is not the shared
`${FRESH_GATE_TARGET}/test-gate`. `gate-steps.txt` is the only step list (spec 146) — edit the test line
there and nowhere else.

## TDD Contract (RED first, pasted)

1. **RED — the artefact property, without any gate run.** A unit test in `xtask/src/fresh_gate.rs` calls the
   new `gate_env` helper and fails while `TEST_GATE_TARGET` is absent (today's shape).
2. **RED — the black-box pair.** `scripts/orch-236-two-runs.sh` runs `fresh-gate` twice on a clean tree at one
   HEAD and exits non-zero when either run is not `PASS`. It is RED today (run 2 fails three `pii-evolve`
   tests), and this is the reading that names the defect.
3. **GREEN.** Both criteria above pass; `scripts/orch-236-no-export-path.sh` (written by the same change)
   finds no `<scratch>/fresh-gate-` string under the shared caches, and the routine launcher lane reads
   5/5.

## Exit Criteria

- [ ] `cargo test -q -p xtask --bins gate_env 2>&1 | grep -qE "test result: ok\. [1-9]"` — the R6 test is
      collected **and** passing (`--bins`, per R3a: `--lib` would collect 0 and still exit 0)
- [ ] `bash scripts/orch-236-no-export-path.sh` exits 0 — no export-baked file in any shared **test-step**
      cache, with its own control proving the check can see a known hit
- [ ] `CARGO_TARGET_DIR=/projects/pi-improved/_tmp/gate-target/test-gate cargo test -q -p pii-evolve --test launcher 2>&1 | grep -qE "test result: ok\. [1-9]"` — the routine launcher lane reads its tests and passes
- [ ] `bash scripts/orch-236-two-runs.sh` exits 0 (two consecutive fresh-gate runs, both `fresh-gate: PASS`)
- [ ] `bash quality-gate.sh` passes in the dev tree

## Guardrails

- `specs/` are READ-ONLY for the implementer: this document is the planner's. Do not edit it; if it is wrong,
  stop and say which requirement does not match the code.
- If the code's shape differs from "The shape of the change" above (no `run_gate`, another env builder), STOP
  and report the mismatch instead of forcing the change.
- Never weaken a test or an assertion to make an item pass — in particular do not touch the three
  `pii-evolve` launcher tests, and do not make `gate-steps.txt` skip the test step.
- Never delete anything outside the export. The per-run test target lives inside the export and is removed
  with it; no shared cache is deleted, and `cargo clean` remains the only way to reclaim one.
- Tests write under `_tmp/` (never `/tmp`) and create the scratch they use (`tmp_scratch`, `create_dir_all`).
- 450-effective-line file gate and `#![forbid(unsafe_code)]` apply as everywhere; `xtask` is no exception.

## Closure

Readings from the orchestrator's own runs on 2026-10-23, all logs under `_tmp/autonomy/logs/`.

- **The worker's change was right and the spec's own criteria were wrong twice.** (a) `cargo test -q -p xtask
  --lib gate_env` collects **0 tests and exits 0** (`running 0 tests … 33 filtered out`) — `xtask` is a binary
  crate and the test lives in `--bins`; the criterion as first written certified nothing (R3a). (b)
  `orch-236-no-export-path.sh` greped the bare prefix over **every** warm cache and found **12098** files on
  the worker's tree, which is why the cycle recorded `verify-failed`: most of those are clippy/build artefacts
  that R2 deliberately keeps on the warm cache, and the rest was the pre-fix residue in `*/test-gate`.
- **The plain door cannot measure an uncommitted fix, and that was measured rather than assumed.** With the
  change in the worktree only, `scripts/orch-236-two-runs.sh` read run 1 `PASS`, run 2 `FAIL` with the same
  three `pii-evolve` launcher tests (`read driver.rs: Os { code: 2 }`) — because `fresh-gate` exports **HEAD**
  with `git archive`, so the export's `gate-steps.txt` did not carry `TEST_GATE_TARGET` while the `xtask`
  binary that set it was built from the dirty tree. The pair poisoned the shared test-step cache with 30121
  export-baked files and the dev-tree gate then failed on them too. **`--worktree` is the pre-commit door:**
  two `fresh-gate --worktree` runs at `f39f61e` both printed `fresh-gate: PASS`, the inventory check passed
  after each, and the dev-tree `quality-gate.sh` that followed read `ALL PASS` with the launcher lane 5/0.
- **The check's pattern was made precise by measurement, with both controls.** Greping the bare prefix flagged
  **three files in a correct tree**: the `xtask` test binary and its incremental artefacts, whose string table
  carries the sample path `<scratch>/fresh-gate-7` used as test data. The pattern is now
  `<scratch>/fresh-gate-[0-9]+/(crates|xtask)/` — a baked manifest dir — and the script runs two controls
  on every invocation: it must see a planted baked manifest dir, and must not flag the sample path.
  **Checker mutation:** planting `_tmp/gate-target/test-gate/debug/deps/orch-fake-export-baked.bin` with the
  string `<scratch>/fresh-gate-424242/crates/pii-evolve/src/driver.rs` read `rc=1`,
  `FAIL, 1 test-step file(s) baked a manifest dir inside an export`; removing the planted file (the only file
  deleted, created by this run) read `rc=0`.
- **Unit mutation (R6):** `TEST_GATE_TARGET` changed from `<export>/_tmp/test-gate` to `warm/test-gate` reddened
  exactly the target assertion — `panicked at xtask/src/fresh_gate.rs:453: lies under the export:
  _tmp/gate-target/test-gate`, `0 passed; 1 failed`. Restored byte-identical (`sha256
  5fc2e1e557ad5a8a22ec3e5ca46bbf6e5d379c0575017d0abc9528a6f6b62f9a` before and after, `touch`ed) and green
  again: `1 passed`.
- **The spec's criteria on the committed tree, `3217788`** (`_tmp/autonomy/logs/orch-236-plain-pair-20261023T072722Z.log`):
  `cargo test -q -p xtask --bins gate_env` 1 passed; `scripts/orch-236-two-runs.sh` → `run 1 PASS`, `run 2 PASS`,
  `two-runs: PASS (both runs at 3217788…)`; `scripts/orch-236-no-export-path.sh` → `PASS (… controls ok …)`;
  launcher lane 5 passed / 0 failed. `bash quality-gate.sh` read `ALL PASS` on the identical tree content
  (`_tmp/autonomy/logs/orch-236-worktree-pair-20261023T065058Z.log`).
- **Cost, stated so it is not re-learned:** the test step no longer reuses binaries, so each fresh-gate
  rebuilds the workspace test targets inside its export. On this host the pair took about 22 minutes for two
  runs (start 07:27:22Z, `c2` at ~07:49Z — `_tmp/autonomy/logs/orch-236-plain-pair-20261023T072722Z.log`
  mtimes), where run 1 built everything and run 2 built its own copy inside its own export. That is the price
  of never reusing a binary that names a directory the previous run deleted.
- Commit paths touched: `xtask/src/fresh_gate.rs`, `gate-steps.txt`, `scripts/orch-236-two-runs.sh`,
  `scripts/orch-236-no-export-path.sh`, this spec, `scripts/autonomy/plan.tsv`.
