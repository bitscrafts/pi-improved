#!/usr/bin/env bash
# Phase 0 quality gate: fmt --check, clippy -D warnings, test.
# Exit 0 only if all three pass.
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
if cargo clippy --workspace --all-targets -- -D warnings; then
    echo "PASS: clippy"
else
    echo "FAIL: clippy"
    status=1
fi

echo "==> cargo test --workspace"
if cargo test --workspace; then
    echo "PASS: test"
else
    echo "FAIL: test"
    status=1
fi

if [ "$status" -eq 0 ]; then
    echo "quality-gate: ALL PASS"
else
    echo "quality-gate: FAILED"
fi
exit "$status"