#!/usr/bin/env bash
# Quality gate: fmt, clippy, test, coverage floor and the xtask repo checks.
# Exit 0 only if every step passes. The steps live in gate-steps.txt (the one list, read by the
# local run and the fresh-gate export alike); scripts/gate-lib.sh runs each with a heartbeat.
# CI-equivalent fresh-tree run (NOT part of this chain; recompiles the workspace):
#   cargo xtask fresh-gate [--worktree] [--overlay src=dest]  (spec 84)
set -uo pipefail

export PATH=/usr/local/cargo/bin:$PATH
cd "$(dirname "$0")" || exit 1

# shellcheck source=scripts/gate-lib.sh
. scripts/gate-lib.sh || exit 1

status=0
steps=0
while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in '' | '#'*) continue ;; esac
    run_step "${line%%|*}" "${line#*|}" || status=1
    steps=$((steps + 1))
done <gate-steps.txt

# An empty or unreadable list must never read as green.
if [ "$steps" -eq 0 ]; then
    echo "FAIL: gate-steps.txt lists no steps"
    status=1
fi

if [ "$status" -eq 0 ]; then
    echo "quality-gate: ALL PASS"
else
    echo "quality-gate: FAILED"
fi
exit "$status"
