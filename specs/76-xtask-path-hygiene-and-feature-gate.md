# Spec: 76 — xtask repo-path hygiene + dependency hygiene (D7, F1/F2)

**Project**: /projects/pi-improved
**Phase**: batch C — 043 §10 items 9 + 13.

## Context
- D7 (043 §4): xtask carries 15 hardcoded `/projects/pi-improved` literals (spec-61
  covered only `crates/`). They break any `git archive` export at another path.
- F1: `llama-cpp-2` is an unconditional dependency of pii-model although it is only
  used behind a compile-time choice; builds without GPU still compile/bind it.
- F2: unused dependencies across workspace members.

## Requirements
R1 (D7). In `xtask/src/`, replace every hardcoded `/projects/pi-improved` literal with
    the existing `workspace_root()` helper (`xtask/src/lib.rs`) or `CARGO_MANIFEST_DIR`-
    derived paths. NO behaviour change beyond path derivation.
R2 (F1). Move `llama-cpp-2` behind a `llamacpp` feature in `crates/pii-model/Cargo.toml`
    (+ optional dependency syntax `dep:llama-cpp-2`), gate every `use llama_cpp_2...`
    and the module containing it with `#[cfg(feature = "llamacpp")]`; downstream code
    that references it must already be feature-isolated — if it is not, add the smallest
    cfg boundary and say so in the report. Default features must NOT include `llamacpp`.
R3 (F2). Remove provably unused deps workspace-wide: `cargo udeps` is NOT installed;
    instead use `RUSTFLAGS="-W unused-crate-dependencies" cargo check --workspace` and
    remove only deps the warnings name on NON-TEST targets (leave test/dev-deps alone).
R4. Verify from a `git archive` export at a DIFFERENT path:
    `cargo check -p xtask` (proves R1), then
    `cargo tree -p pii-model | grep -c llama` = 0 without the feature.

## Exit Criteria
- [ ] `[ "$(grep -rn '\"/projects/pi-improved' xtask/src | wc -l)" = 0 ]`
- [ ] `cargo check -p xtask` exits 0 from `/projects/_tmp/<scratch>/` of a git archive export
- [ ] `[ "$(cargo tree -p pii-model 2>/dev/null | grep -c llama)" = 0 ]`
- [ ] `cargo tree -p pii-model --features llamacpp 2>/dev/null | grep -c llama | grep -v '^0'`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` exits 0
- [ ] `cargo test -p xtask` exits 0
- [ ] `cargo test -p pii-model` exits 0 (feature-off default build compiles + tests)

## Guardrails
- Do NOT add a `llamacpp` default feature anywhere (default build must not link llama).
- Do NOT touch `crates/pii-server/` runtime code for D7 (xtask-only).
- Do NOT run `cargo fix`/fmt sweeps beyond the touched files.
- Do NOT commit; leave the tree for the orchestrator.
- Scratch exports under `/projects/_tmp/` only.