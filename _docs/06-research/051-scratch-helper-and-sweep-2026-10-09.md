# Spec 96 scratch report
Bare `tempdir_in(<_tmp literal>)` sites (measured by the new lint on the pre-change tree): **18** before, **0** after.
Total `tempdir_in` calls on a `_tmp` path incl. helper-guarded ones: ~31 before; the 13 others already had a `create_dir_all` in a helper/fn.

Helper: `pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), prefix) -> io::Result<TempDir>` (crates/pii-store/src/testing.rs; `testing` made `pub mod`; `tempfile` moved dev-dep -> dep of pii-store).

## Converted to the helper (pii-server, which depends on pii-store) — 12 sites
tests/peercred.rs, tests/memory_rpc.rs, tests/latency.rs, tests/handshake.rs (x4), src/node_kinds/nodes/gate.rs, src/node_kinds/nodes/heal.rs, src/engine/runs.rs (x3)
## Converted to explicit create_dir_all before the call (crate does not depend on pii-store; adding the dep for a test helper is not justified) — 3 files, 3 sites
pii-model/tests/ollama_http.rs (1), pii-evolve/tests/launcher.rs (2)
(heal.rs: removed the now-unused `use tempfile::Builder`.)
## Remaining `tempdir_in(..._tmp...)` bare sites: 0
Sites left unchanged because already guarded (create_dir_all in the same fn or in the helper `tmp_root()/scratch_root()` etc.): pii-memory, pii-graph, pii-tools, pii-core, pii-mcp, pii-model/src/lib.rs, pii-store (heal.rs, mem/write.rs), pii-server (reattach, heartbeat, inline_measure, daemon/{mod,router,lag}, worktree_join_containment), pii-evolve launcher:118.

## Enforcement
`cargo run -q -p xtask -- lint-scratch` (xtask/src/lint_scratch.rs, 4 unit tests), wired into quality-gate.sh after lint-files as a failing step.
Teeth (export <scratch>/spec96/export): reintroduced bare pattern in peercred.rs -> exit 1 (`peercred.rs:42 ... no create_dir_all/tmp_scratch`); restored -> exit 0.

## Proofs
- `cargo run -q -p xtask -- fresh-gate --worktree` -> `fresh-gate: PASS` (exit 0; log fresh-gate.log)
- No-_tmp control: export of the working tree (git ls-files -co), `find crates xtask -name _tmp` found none (gitignored; nothing to move aside), `cargo build --workspace && cargo test --workspace --lib --bins --tests` -> exit 0, 65 "test result: ok", no failures (control-test.log).
- `bash quality-gate.sh` -> exit 0, all steps PASS incl. lint-scratch.
- lint-files: `190 files checked, max effective 373 lines (crates/pii-store/src/lib.rs)` — no warning (threshold 380).
