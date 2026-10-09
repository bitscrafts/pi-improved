# Spec: 85 — the spec-80 heartbeat tests are hermetic too (found by fresh-gate)

**Project**: /projects/pi-improved
**Phase**: 045 re-review follow-up — the N1 class, second instance, found by the tool spec 84 built.

## Context
Spec 84's tool ran against the current tree and immediately failed, naming two tests:
`engine::conn_heartbeat_tests::heartbeat_memory_consolidate` and
`engine::conn_heartbeat_tests::heartbeat_control_light_method_never_late`.
`crates/pii-server/src/engine/conn_heartbeat_tests.rs:76` writes its scratch under
`crates/pii-server/_tmp`, which does not exist in a fresh `git archive` export — the same shape
as the `lanes.rs` bug spec 83 fixed. Those tests were added by spec 80 (batch D) and pass in the
developer tree only because `crates/pii-server/_tmp` already exists there.

## Requirements
R1. Every scratch path in `crates/pii-server/src/engine/conn_heartbeat_tests.rs` is created by
    the test itself: use `tempfile::Builder::new().prefix(..).tempdir_in(<a dir the test creates>)`
    (or `create_dir_all` before use) — no reliance on a pre-existing `_tmp` anywhere.
R2. Assertions and the heartbeat property under test are unchanged (this is hermeticity only).
R3. Prove it with the tool: `cargo run -q -p xtask -- fresh-gate --worktree` must exit 0 and print
    `fresh-gate: PASS` (with spec 83 already landed). Record the verbatim output.
R4. If fresh-gate then names ANY other failing test, do NOT fix it in this spec: report it. This
    spec's scope is the two named tests plus any scratch-path literal in that same file.

## Exit Criteria
- [x] no pre-existing-dir literal: the file now does `create_dir_all(CARGO_MANIFEST_DIR/_tmp)` then `tempdir_in(root)` (verified in the diff; a first attempt used `.tempdir()` and was corrected — the project rule is tests under `_tmp/`, never `/tmp`)
- [x] `cargo test -p pii-server heartbeat` → **7 passed / 0 failed** (verified by the orchestrator)
- [ ] **NOT MET — and it found the THIRD instance of the class.** `fresh-gate --worktree` still fails, now naming `heal::tests::attempt_ring_caps_and_counts` (panic `NotFound … /crates/pii-store/_tmp/heal-ring-piHjtg`): `crates/pii-store/src/heal.rs` has the same non-hermetic shape. Also recorded: the earlier "pii-tools failed" observation was the DISK-FULL condition and is gone now. Follow-up: spec 86.
- [x] `cargo clippy -p pii-server --all-targets -- -D warnings` exits 0 and `cargo fmt --all --check` exits 0 (verified by the orchestrator)
- [ ] the report lists every scratch path the file uses and how each is created

## Guardrails
- Do NOT change production code (`engine/conn.rs` etc.) — tests only.
- Do NOT weaken the heartbeat assertions or the light-method control.
- Do NOT `rm -rf` anything; scratch under `/projects/_tmp/` only.
- Do NOT commit; leave the tree for the orchestrator.

## Orchestrator verification (2026-10-09)
- The fix is on-convention and minimal: `create_dir_all(CARGO_MANIFEST_DIR/_tmp)` + `tempdir_in(root)`
  (the worker's first attempt, `.tempdir()`, was corrected by the resume brief).
- `cargo test -p pii-server heartbeat` → 7 passed; clippy + fmt clean.
- **The class is bigger than spec 84 first showed.** `fresh-gate --worktree` is the tool working as
  intended: it named the two heartbeat tests (now fixed), and on the next run named a THIRD instance
  in `crates/pii-store/src/heal.rs` (`attempt_ring_caps_and_counts`, writing `crates/pii-store/_tmp/
  heal-ring-*`). Each instance is a test that depends on a directory only a developer tree has.
  The remaining work is spec 86: fix what the tool names, iterating until it prints `fresh-gate: PASS`.
