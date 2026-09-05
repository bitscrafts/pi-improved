#!/usr/bin/env bash
# Pii MVP verification suite (2026-09-05).
#
# Exercises the actually-running system end-to-end (not just `cargo test`):
# workspace build/lint/unit-tests, the live lnx daemon, a real model call
# through it, the self-healing ledger, the human-approval gate, and the
# rss-gate soak. Prints PASS/FAIL per check and a final summary; exits
# non-zero if anything failed.
#
# Usage: ./mvp-verify.sh [--skip-soak]   (soak takes ~5 minutes)
set -uo pipefail

REPO=/projects/pi-improved
BIN=/root/.local/bin/pii
RUNTIME=/root/.pii/runtime
SKIP_SOAK=0
[ "${1:-}" = "--skip-soak" ] && SKIP_SOAK=1

PASS=0
FAIL=0
FAILED_NAMES=()

check() {
    local name="$1"; shift
    if "$@" > /tmp/mvp-verify-out.$$ 2>&1; then
        echo "PASS: $name"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $name"
        sed 's/^/    /' /tmp/mvp-verify-out.$$ | tail -15
        FAIL=$((FAIL + 1))
        FAILED_NAMES+=("$name")
    fi
    rm -f /tmp/mvp-verify-out.$$
}

export PATH=/usr/local/cargo/bin:$PATH
cd "$REPO" || exit 1

echo "== 1. Build & static checks =="
check "workspace builds (debug, all targets)" cargo build --workspace --all-targets
check "workspace builds (release, pii-cli)" cargo build --release -p pii-cli
check "cargo fmt --check" cargo fmt --all --check
check "cargo clippy -D warnings (workspace, all targets)" \
    cargo clippy --workspace --all-targets -- -D warnings

echo
echo "== 2. Unit test suite =="
check "cargo test --workspace" cargo test --workspace

echo
echo "== 3. xtask gates =="
check "xtask lint-files" cargo xtask lint-files
check "xtask deps-check" cargo xtask deps-check
check "xtask net-check" cargo xtask net-check
check "xtask reachability-check" cargo xtask reachability-check
check "xtask doc-drift-check" cargo xtask doc-drift-check

echo
echo "== 4. Graph lint (all starter graphs except known-bad crash.toml) =="
for g in graphs/starter/*.toml; do
    [ "$(basename "$g")" = "crash.toml" ] && continue
    check "graph lint: $(basename "$g")" "$BIN" graph lint "$g"
done

echo
echo "== 5. Live daemon =="
check "installed binary runs (--version)" "$BIN" --version
check "daemon process is alive" pgrep -f "$BIN daemon"
check "sys status reachable" bash -c "PII_RUNTIME_DIR=$RUNTIME $BIN sys status"

echo
echo "== 6. Live model call (real end-to-end LLM turn, not scripted:) =="
RUN_JSON=$(PII_RUNTIME_DIR=$RUNTIME "$BIN" run start assistant \
    --input '{"prompt":"Say exactly: MVP_VERIFY_OK"}' 2>&1)
RUN_ID=$(echo "$RUN_JSON" | python3 -c "import sys,json; print(json.load(sys.stdin)['run_id'])" 2>/dev/null)
if [ -n "$RUN_ID" ]; then
    echo "PASS: run start assistant (run_id=$RUN_ID)"
    PASS=$((PASS + 1))
else
    echo "FAIL: run start assistant"
    echo "    $RUN_JSON"
    FAIL=$((FAIL + 1))
    FAILED_NAMES+=("run start assistant")
fi

if [ -n "$RUN_ID" ]; then
    STATUS=""
    for _ in $(seq 1 20); do
        for S in succeeded failed; do
            R=$(PII_RUNTIME_DIR=$RUNTIME "$BIN" run ls --status "$S" 2>/dev/null)
            if echo "$R" | grep -q "\"run_id\":\"$RUN_ID\""; then
                STATUS="$S"
                break 2
            fi
        done
        sleep 3
    done
    if [ "$STATUS" = "succeeded" ]; then
        echo "PASS: live model run reached 'succeeded' (real LLM call worked)"
        PASS=$((PASS + 1))
    else
        echo "FAIL: live model run did not succeed (status=${STATUS:-timeout})"
        FAIL=$((FAIL + 1))
        FAILED_NAMES+=("live model run succeeded")
    fi
fi

echo
echo "== 7. Self-healing ledger (Slice 0/1) =="
check "heal ls returns valid JSON" bash -c "PII_RUNTIME_DIR=$RUNTIME $BIN heal ls | python3 -m json.tool > /dev/null"

echo
echo "== 8. Approval gate =="
check "approve ls returns valid JSON" bash -c "PII_RUNTIME_DIR=$RUNTIME $BIN approve ls | python3 -m json.tool > /dev/null"

echo
if [ "$SKIP_SOAK" -eq 0 ]; then
    echo "== 9. rss-gate soak (300s, ~5 min) =="
    check "rss-gate (max_kb<=50000, slope_kb<=16384)" \
        cargo xtask rss-gate --secs 300 --interval 10 --max-mb 50 --runs 3000
else
    echo "== 9. rss-gate soak SKIPPED (--skip-soak) =="
fi

echo
echo "=================================================="
echo "RESULT: $PASS passed, $FAIL failed"
if [ "$FAIL" -gt 0 ]; then
    echo "Failed checks:"
    for n in "${FAILED_NAMES[@]}"; do echo "  - $n"; done
    exit 1
fi
exit 0
