# Spec 286: a committed mutation sweep over the composition code

## Overview
**Project**: <repo>
**Phase**: production — composability, level 3 of 4. Every defect found in this work was found by a **mutation I wrote by hand and kept in `_tmp/`**: m5 (spec 271, a vacuous claude assertion), m9 (spec 276, the coder's gate unreachable in a graph run), d5 (spec 273, `GATE_WRITE_DIRS` ignored). Those scripts are not in the repo, so the next change to `call.rs` is protected only by whoever remembers to rerun them. This spec commits the sweep: a table of named mutations of the composition code, applied one at a time to a **scratch copy of the files**, each required to turn a named test file red, with the originals restored byte-identically and checked by digest.

## Verified Premises
- **CONFIRMED** (read 2026-10-26) the composition source: `fragment/call.rs` (437 lines), `call_needs.rs` (234), `call_record.rs` (76), `call_rewrite.rs` (67), `weave.rs` (161), `nodes_require.rs` (102), `run/call_events.rs` (22), `coder/budget.rs` (75), `coder/graph_node.rs`.
- **CONFIRMED** no mutation tool is installed (`cargo-mutants` absent) and none is wanted: the sweep is a table, because the useful mutations are the **semantic** ones (drop the pin check, drop the `needs` refusal, share nothing through the call), which a generic operator-flipper does not choose.
- **CONFIRMED** the method that has worked: mutate a **scratch copy**, point the test at it, never edit the live file while a queue cycle may be reading it (the unlisted-changes guard and the gate both read the live tree). Spec 275's test did this for `cycle.sh`; the approach for Rust is a throwaway **git worktree** (`git worktree add` under `_tmp/`), mutate there, run the one test file there, remove the worktree with `git worktree remove`.
- **MEASURE FIRST** that a second worktree builds in reasonable time with `CARGO_TARGET_DIR` shared (the repo's shared target dir has a known fragility: `xtask` bakes `CARGO_MANIFEST_DIR`, `xtask/src/lib.rs:31`). If a worktree build is too slow or fails for that reason, the fallback is a **scratch copy of the crate source tree** under `_tmp/`; record which was used and why.
- **MEASURE FIRST** each mutation's anchor: every `old` string must occur **exactly once** in its file (the helper asserts this and refuses otherwise), so a refactor that moves a line turns the sweep red instead of silently skipping.

## Requirements
### Functional
- **R1. `scripts/mutate/composition.sh`** (committed, executable, `set -euo pipefail`, all paths relative to the repo root, no absolute path) plus `scripts/mutate/composition_mutations.py` (the table and the applier, run with `uv run --no-project python`). The table has **at least 18** rows `id | file | old | new | test-file | kind`, `kind` being `required` (must turn the test red) or `probe` (reported, never counted as a pass). Required mutations, one per behaviour: (1) the pin check passes any digest, (2) the pin check is skipped when the digest is absent, (3) a sub-graph node read of a host node is allowed (`graph.call.leak` removed), (4) an undeclared `needs` key loads, (5) a null `needs` key does not refuse, (6) `MAX_DEPTH` raised by one, (7) the self-call/cycle check removed, (8) `caps` ceiling not enforced, (9) the call record is not attached, (10) `graph.call` is not journalled, (11) the inlined node ids lose their call prefix (id collision), (12) the call's `in` mapping is ignored, (13) the sub-graph exit port is not wired back to the host, (14) the coder never charges the run budget, (15) the coder's start gate removed, (16) the per-node session path ignores the node id, (17) the daemon path builds the budget without `[limits]`, (18) a `call` inside a fragment accepted. Each names the test file that must catch it (`call_inlines_a_graph`, `call_needs`, `call_records`, `run_journals_calls`, `coder_charges_run_budget`, `coder_session_per_node`, `spec_graph_runs_through_the_daemon`, `composition_failure_matrix`, …).
- **R2. Behaviour per mutation:** copy the target file into the scratch tree, apply, run **only** the named test file, record `RED` / `SURVIVED` / `NO-APPLY`, restore, and **compare the sha256 of the restored file to the original's**; a mismatch aborts the whole sweep with a nonzero exit and a message naming the file. Output is one row per mutation and a final line `MUTATION-SWEEP: required <red>/<total> probes <n>`; exit 0 only when **every required row is RED** and every digest matched.
- **R3. The sweep tests itself.** `scripts/mutate/tests/composition-sweep-test.sh` runs the sweep against a **fixture** — a tiny crate under `scripts/mutate/tests/fixture/` made of exactly three files: `Cargo.toml` (carrying an empty `[workspace]` table so the crate detaches from the repo workspace and builds on its own), `.gitignore` (naming `Cargo.lock` and `target/`, so building the fixture leaves no unlisted path in the tree) and `src/lib.rs` (one function and one test) — with three table rows: a mutation the test catches (RED), one it does not (SURVIVED), and one whose anchor is absent (NO-APPLY) — and asserts the sweep reports exactly those three, exits nonzero, and left the fixture byte-identical. CONTROL: with the SURVIVED row removed the sweep exits 0.
- **R4. First run, recorded.** The worker runs the real sweep once and writes the result table to `_docs/05-operations/COMPOSITION-MUTATION-SWEEP.md` (the date, the commit, one row per mutation). **A SURVIVED required row is a finding**: it is listed under "Survivors" with the mutation, and the worker does **not** write a test to kill it in this spec — that is the next spec's work, named in the document.
- **R5. Wiring.** The sweep is NOT in the per-commit gate (it builds and runs many tests). It is invoked by `scripts/autonomy/` only through an explicit `PII_SWEEP=1` flag — **out of scope here**; this spec only makes it runnable and proven.

### Non-Functional
- No change under `crates/*/src`. The live tree is never edited (the sweep works in the scratch tree); a leftover scratch tree from an interrupted run is moved under `_tmp/_trash/`, never `rm -rf`'d. Every file under the 450-line gate. Python through `uv run`.

## Exit Criteria
- [ ] `test -x scripts/mutate/composition.sh && bash -n scripts/mutate/composition.sh`
- [ ] `bash scripts/mutate/tests/composition-sweep-test.sh 2>&1 | tail -1 | grep -q "FAIL: 0"`
- [ ] `test "$(grep -c '^m[0-9]' scripts/mutate/composition_mutations.py)" -ge 18`
- [ ] `grep -q 'sha256' scripts/mutate/composition.sh`
- [ ] `grep -qE 'MUTATION-SWEEP: required' _docs/05-operations/COMPOSITION-MUTATION-SWEEP.md`
- [ ] `git diff --quiet -- crates/pii-graph/src crates/pii-server/src graphs`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

## Files
- `scripts/mutate/composition.sh`
- `scripts/mutate/composition_mutations.py`
- `scripts/mutate/tests/composition-sweep-test.sh`
- `scripts/mutate/tests/fixture/Cargo.toml`
- `scripts/mutate/tests/fixture/.gitignore`
- `scripts/mutate/tests/fixture/src/lib.rs`
- `_docs/05-operations/COMPOSITION-MUTATION-SWEEP.md`

## Guardrails
- Do NOT edit any source file in the live tree. Do NOT use `rm -rf` (move to `_tmp/_trash/`). Do NOT weaken a mutation to make it turn red. Do NOT write tests to kill a survivor here. Do NOT commit.
- If a required mutation SURVIVES, report it; do not delete the row.

## Follow-ups
A spec that kills each survivor; the `PII_SWEEP=1` hook in the queue's cycle; the sweep for `agent_quota.rs` and `cycle.sh`.
