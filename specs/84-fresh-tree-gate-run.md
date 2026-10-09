# Spec: 84 — a fresh-tree gate run, so the N1 class cannot recur

**Project**: /projects/pi-improved
**Phase**: 045 re-review, candidate 2 (the reviewer pairs it with 83).

## Context
The gate was green only because `crates/pii-memory/_tmp/phase3` happened to exist in the
developer tree (untracked). A gate that depends on leftover state is the failure mode the
gate exists to prevent — and disposable worktrees are exactly how the coding harness runs.
Measured on today's HEAD: in a `git archive` export, `cargo test -p pii-memory` is RED.

## Requirements
R1. A new xtask subcommand, e.g. `cargo xtask fresh-gate`, that:
    a. exports the CURRENT tree with `git archive HEAD` (or `git stash create`-free equivalent
       — it must not touch the working tree) into a unique dir under `/projects/_tmp/fresh-gate-<pid>/`,
    b. overlays the WORKING TREE's uncommitted changes when `--worktree` is passed (so it can
       gate work in progress, not only committed state),
    c. runs the gate inside that export with an isolated `CARGO_TARGET_DIR`,
    d. exits non-zero on failure and prints which step failed and the failing test names,
    e. never deletes the export on failure (leave it for inspection), and cleans it up ONLY on
       success (and only what it created — the dir must match its own `fresh-gate-*` prefix).
R2. Document it in `quality-gate.sh`'s header comment and in HANDOFF (one line each): it is the
    CI-equivalent run, not part of the default gate (it recompiles the workspace).
R3. A test/exit criterion proving it FAILS on the pre-83 tree and PASSES after 83.

## Exit Criteria
- [ ] `cargo run -q -p xtask -- fresh-gate --help 2>&1 | grep -q 'fresh'` exits 0 (subcommand exists)
- [ ] On the CURRENT tree (spec 83 NOT yet applied): `cargo run -q -p xtask -- fresh-gate` exits NON-ZERO and its output names at least one of `degrades_honestly_without_embedder`, `graph_lane_recalls_doc_via_shared_concept`, `memory_hit_carries_doc_text` — record this output in the report as the red proof
- [ ] With spec 83 applied to the same export (`--worktree`), the same command exits 0
- [ ] `cargo test -p xtask` exits 0 and `cargo clippy --workspace --all-targets -- -D warnings` exits 0
- [ ] `cargo run -q -p xtask -- lint-files` exits 0 with no new warning
- [ ] the export directory it created is listed in the report (and, on the red path, left in place)

## Guardrails
- Do NOT add the fresh-tree run to the default `quality-gate.sh` chain (it recompiles everything;
  it is a CI/weekly check, available on demand).
- Do NOT touch `_repos/`, `specs/`, or any test source other than what R1 requires.
- Never `rm -rf` a path the command did not itself create in this run; a guard around any
  cleanup must REFUSE on an empty/unexpected variable (fail-safe), per the project's hard rule 0.
- Do NOT commit; leave the tree for the orchestrator.