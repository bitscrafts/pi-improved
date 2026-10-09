# Spec: 84 — a fresh-tree gate run, so the N1 class cannot recur

**Project**: /projects/pi-improved
**Phase**: 045 re-review, candidate 2 (the reviewer pairs it with 83).

## Context
The gate was green only because `crates/pii-memory/_tmp/phase3` happened to exist in the
developer tree (untracked). A gate that depends on leftover state is the failure mode the
gate exists to prevent — and disposable worktrees are exactly how the coding harness runs.
Measured on today's HEAD: in a `git archive` export, `cargo test -p pii-memory` is RED.

## Requirements
R1. A new xtask subcommand, e.g. `cargo xtask fresh-gate`, that (with `--worktree` overlaying the
    current working tree, and an optional `--overlay src=dest` for a single file):
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
- [x] `cargo run -q -p xtask -- fresh-gate --help` works (subcommand exists: `[--worktree] [--overlay src=dest]`)
- [ ] RED PROOF (spec 83 has ALREADY landed in HEAD, so the red must be produced, not found):
      make a scratch copy of `crates/pii-memory/tests/lanes.rs` with the three tests reverted to
      hardcoded `_tmp/phase3` paths (pre-83 shape), run
      `cargo run -q -p xtask -- fresh-gate --overlay <dir>/lanes.rs=crates/pii-memory/tests/lanes.rs`
      (or the equivalent the command offers) and show it exits NON-ZERO naming at least one of
      `degrades_honestly_without_embedder`, `graph_lane_recalls_doc_via_shared_concept`,
      `memory_hit_carries_doc_text`. Record verbatim in the report. If the command offers no overlay,
      use `--worktree` against a scratch checkout whose lanes.rs is the pre-83 copy.
- [ ] **NOT MET YET — and the tool is right to say so.** With spec 83 applied, `fresh-gate --worktree` still FAILS, naming TWO OTHER tests: `engine::conn_heartbeat_tests::heartbeat_memory_consolidate` and `engine::conn_heartbeat_tests::heartbeat_control_light_method_never_late` — a SECOND instance of the N1 class, in the file spec 80 added (`conn_heartbeat_tests.rs:76` writes under `crates/pii-server/_tmp`). The tool found it on its first green run. Orchestrator reproduced it independently (export kept at `/projects/_tmp/fresh-gate-2931282`). Now spec **85**.
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

## Orchestrator verification (2026-10-09)
- **Destructive-safety reviewed at the source**: `fresh_gate::safe_to_remove(base, dir)` refuses
  unless base is non-empty, the dir's name starts with the prefix AND is longer than it, and the
  dir's parent is exactly `base`; `remove_dir_all` runs ONLY on the success path. On failure the
  export is kept for inspection. That is fail-safe, as the guardrail required.
- **The red proof exists and is real**: the worker ran the pre-83 `lanes.rs`
  (`git show ee38646^:...`) through `--overlay` and got
  `fresh-gate FAILED; failed steps: ["test (lib/bins/tests)"]; failing tests:
  ["graph_lane_recalls_doc_via_shared_concept", "degrades_honestly_without_embedder",
  "memory_hit_carries_doc_text"]` — the N1 class, named.
- **My own green run** (`cargo xtask fresh-gate --worktree`) reproduced the tool's second finding
  verbatim: `failing tests: ["engine::conn_heartbeat_tests::heartbeat_memory_consolidate",
  "engine::conn_heartbeat_tests::heartbeat_control_light_method_never_late"]`.
- So spec 84 is DONE as a tool (subcommand, red proof, failure naming, fail-safe cleanup,
  docs) and its criterion 3 is an honest, current FAIL that spec 85 closes. The tool's first
  green run finding a defect in the batch I shipped yesterday is the point of the tool.
