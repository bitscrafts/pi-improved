#!/usr/bin/env bash
# Phase 0 quality gate: fmt --check, clippy -D warnings, test.
# Exit 0 only if all three pass.
# CI-equivalent fresh-tree run (NOT part of this chain; recompiles the workspace):
#   cargo xtask fresh-gate [--worktree] [--overlay src=dest]  (spec 84)
set -uo pipefail

export PATH=/usr/local/cargo/bin:$PATH
cd "$(dirname "$0")" || exit 1

status=0

echo "==> cargo fmt --all --check"
if cargo fmt --all --check; then
    echo "PASS: fmt"
else
    echo "FAIL: fmt"
    status=1
fi

echo "==> cargo clippy --workspace --all-targets -- -D warnings"
# F-01 fix: clippy's --all-targets build writes libschemars/libserde_json
# (and other proc-macro-generated) rlibs into target/debug/deps with a
# feature-unification hash that differs from the plain `cargo test` build.
# rustdoc's doctest linker then picks up the stale/mismatched rlib and fails
# with E0463 ("can't find crate"). Isolating clippy into its own target dir
# means it can never poison the artifacts `cargo test --workspace` (which
# includes the doctest step) relies on.
if CARGO_TARGET_DIR=target/clippy-gate cargo clippy --workspace --all-targets -- -D warnings; then
    echo "PASS: clippy"
else
    echo "FAIL: clippy"
    status=1
fi

echo "==> cargo clippy -p pii-model --features llamacpp --all-targets -- -D warnings"
# Spec 89 (N4): `llamacpp` is off in every other step (spec 76 keeps llama out
# of the default build), so the in-process llama provider and the tests that
# carry required-features = ["llamacpp"] would otherwise be compiled by no
# gate step. Compile + lint only; the real-model tests stay #[ignore]d. Own
# target dir: the feature changes unification hashes, same hazard as F-01.
if CARGO_TARGET_DIR=target/llamacpp-gate cargo clippy -p pii-model --features llamacpp --all-targets -- -D warnings; then
    echo "PASS: clippy (llamacpp)"
else
    echo "FAIL: clippy (llamacpp)"
    status=1
fi

echo "==> cargo test --workspace"
# pii-worker's supervisor integration tests spawn the real `pii` binary at
# target/debug/pii (a fixed path, not a `CARGO_BIN_EXE_pii` env lookup), so
# it must exist before `cargo test` runs it.
if cargo build --workspace; then
    :
else
    echo "FAIL: build"
    status=1
fi
if cargo test --workspace --lib --bins --tests; then
    echo "PASS: test (lib/bins/tests)"
else
    echo "FAIL: test (lib/bins/tests)"
    status=1
fi

echo "==> cargo test --workspace --doc"
# F-01: doctests build each crate's lib WITHOUT `cfg(test)`, while the step
# above just built the same libs WITH `cfg(test)` (for their own unit
# tests). Both compilations share target/debug/deps; rustdoc's doctest
# linker can then pick up a stale/mismatched rlib hash for a
# proc-macro-heavy dep (schemars/serde_json), failing with E0463. Running
# the doctest step in its own CARGO_TARGET_DIR (per the 026 report's
# recommendation: "run cargo test --workspace --doc as its own step with a
# fresh profile") means it always builds its own non-cfg(test) libs from
# scratch and never races the previous step's artifacts.
if CARGO_TARGET_DIR=target/doctest-gate cargo test --workspace --doc; then
    echo "PASS: test (doc)"
else
    echo "FAIL: test (doc)"
    status=1
fi

echo "==> cargo llvm-cov (coverage floor, spec 108)"
# Spec 108 R2: line coverage of the lib/bins/tests scopes (no doctests: those
# need nightly) must stay at or above the floor pinned in coverage-floor.txt.
# Needs cargo-llvm-cov + the rustup llvm-tools component; their absence is a
# FAIL, not a skip. Own target dir (target/llvm-cov-target) so instrumented
# artifacts never touch the plain build.
COV_JSON=target/coverage-summary.json
if cargo llvm-cov --workspace --lib --bins --tests --json --summary-only --output-path "$COV_JSON" >/dev/null \
    && cargo run -q -p xtask -- coverage-floor "$COV_JSON"; then
    echo "PASS: coverage-floor"
else
    echo "FAIL: coverage-floor"
    status=1
fi

echo "==> cargo run -q -p xtask -- lint-files"
# 450-effective-line file gate (spec 67, Fix 1): pure text scan, no
# target-dir isolation needed. Also enforced here so pidag runs whose
# exit criteria omit lint-files still get the gate.
if cargo run -q -p xtask -- lint-files; then
    echo "PASS: lint-files"
else
    echo "FAIL: lint-files"
    status=1
fi

echo "==> cargo run -q -p xtask -- lint-scratch"
# Spec 96: a tempdir_in on a crate-local `_tmp` directory must be preceded by
# create_dir_all / tmp_scratch in the same fn (the N1 class: green in a
# developer tree where _tmp is left over, NotFound in a fresh clone).
if cargo run -q -p xtask -- lint-scratch; then
    echo "PASS: lint-scratch"
else
    echo "FAIL: lint-scratch"
    status=1
fi

echo "==> cargo run -q -p xtask -- lint-observability"
# Spec 98: every crate depends on and uses the pii-observe seam (or carries a
# named exemption), no new raw prints outside the allowlist, and every name in
# the registry is listed in ALL.
if cargo run -q -p xtask -- lint-observability; then
    echo "PASS: lint-observability"
else
    echo "FAIL: lint-observability"
    status=1
fi

echo "==> cargo run -q -p xtask -- reachability-check"
# Declared-capability caller gate (spec 72, D3 fix): every seed entry's
# needle must have a REAL external caller — outside the declaring crate,
# outside comments and cfg(test) bodies. Runs AFTER the test steps and
# BEFORE the summary so a deleted caller turns the shared gate red.
if cargo run -q -p xtask -- reachability-check; then
    echo "PASS: reachability-check"
else
    echo "FAIL: reachability-check"
    status=1
fi

if [ "$status" -eq 0 ]; then
    echo "quality-gate: ALL PASS"
else
    echo "quality-gate: FAILED"
fi
exit "$status"