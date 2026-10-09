# Spec: 86 — fresh-gate stays affordable, and the tree reaches a real fresh PASS

**Project**: /projects/pi-improved
**Phase**: 045 re-review follow-up — the N1 class, swept to completion; plus the tool hygiene the
disk-full incident of 2026-10-09 proved is missing.

## Context
`cargo xtask fresh-gate` (spec 84) keeps EVERY failed export — 8.1-8.3 GB each. Five of them filled
the 594G volume to 0 bytes free, which silently killed a worker mid-run. It also has no free-space
check, so it will happily start an export it cannot afford. And it has now named three instances of
one class (a test writing under a crate-local `_tmp/` that only exists in a developer tree):
`pii-memory/tests/lanes.rs` (fixed, spec 83), `pii-server/src/engine/conn_heartbeat_tests.rs` (fixed,
spec 85), and `crates/pii-store/src/heal.rs` (`attempt_ring_caps_and_counts`, still open).

## Requirements
R1 (tool). In `xtask/src/fresh_gate.rs`:
    a. keep AT MOST ONE failed export: before creating a new one, remove any existing
       `<base>/fresh-gate-*` that is NOT the one about to be created — using the existing
       `safe_to_remove` guard, never a bare `rm -rf`;
    b. refuse to start when free space at the export base is below 2 GB (read it with `statvfs`;
       no new dependency — `nix` is already a dependency of the workspace, or call `df` and parse).
       The refusal must say how much space is free and name the env var to override it
       (`FRESH_GATE_MIN_FREE_MB`, default 2048);
    c. print the export path and the free space in MB at start.
R2 (tests). Fix every remaining non-hermetic scratch path the tool names, one at a time, until
    `cargo run -q -p xtask -- fresh-gate --worktree` prints `fresh-gate: PASS`. Known open offender
    to start with: `crates/pii-store/src/heal.rs` (the tests around `heal-ring-*`). Do NOT fix by
    creating directories from production code — tests must create what they need
    (`create_dir_all` then `tempdir_in`, the spec-83 pattern).
R3 (definition of done). The final state has a green fresh-tree gate AND a repo-tree gate; report
    both verbatim.

## Exit Criteria
- [ ] `[ "$(grep -c 'FRESH_GATE_MIN_FREE_MB' xtask/src/fresh_gate.rs)" -ge 1 ]`
- [ ] `[ "$(grep -c 'safe_to_remove' xtask/src/fresh_gate.rs)" -ge 2 ]` (cleanup + retention both guarded)
- [ ] retention proof: run fresh-gate twice in a row, each failing, and show that at most one `fresh-gate-*` dir remains — with the survivor being the newer one
- [ ] free-space proof: `FRESH_GATE_MIN_FREE_MB=999999 cargo run -q -p xtask -- fresh-gate` REFUSES (non-zero, names the free space), spends no build time, and creates NO export
- [ ] `cargo run -q -p xtask -- fresh-gate --worktree` prints `fresh-gate: PASS`
- [ ] `bash quality-gate.sh` exits 0 in the repo tree
- [ ] `cargo test -p xtask` and `cargo clippy --workspace --all-targets -- -D warnings` exit 0
- [ ] the report lists each test file changed in R2 and the exact panic the tool had named for it

## Guardrails
- HARD RULE 0: any directory removal must be guarded by `safe_to_remove` (prefix + parent + non-empty
  base). Never `rm -rf "$VAR"` on anything not created by this tool in this run.
- Do NOT change what the gate itself runs; this spec does not add steps to `quality-gate.sh`.
- Do NOT weaken or delete any test assertion. Hermeticity only.
- Do NOT commit; leave the tree for the orchestrator.
