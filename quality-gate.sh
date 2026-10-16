#!/usr/bin/env bash
# Quality gate: fmt, clippy, test, coverage floor and the xtask repo checks.
# Exit 0 only if every step passes. The steps live in gate-steps.txt (the one list, read by the
# local run and the fresh-gate export alike); scripts/gate-lib.sh runs each with a heartbeat.
# CI-equivalent fresh-tree run (NOT part of this chain; recompiles the workspace):
#   cargo xtask fresh-gate [--worktree] [--overlay src=dest]  (spec 84)

# The llamacpp step builds llama.cpp (C++) from source. Measured 2026-10-16: with no job setting it ran only
# 4 clang++ on a 32-core host, which is why a cold gate took 25+ minutes and then hit its timeout. nproc is
# the parallelism this machine actually has.
export CMAKE_BUILD_PARALLEL_LEVEL="${CMAKE_BUILD_PARALLEL_LEVEL:-$(nproc)}"
set -uo pipefail

export PATH=/usr/local/cargo/bin:$PATH
cd "$(dirname "$0")" || exit 1

# shellcheck source=scripts/gate-lib.sh
. scripts/gate-lib.sh || exit 1

# Spec 155: what this gate runs but nothing has exercised. Printed by `--summary` (which runs
# nothing) and at the end of every run, so a passing hal-gpiod step cannot read as hardware coverage.
unexercised_summary() {
    echo "clippy(hal-gpiod) and the hal-gpiod feature steps are UNEXERCISED: 117 lines that have never run on hardware; a PASS there is compile + lint only, not coverage"
}
if [ "${1:-}" = "--summary" ]; then
    unexercised_summary
    exit 0
fi

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

unexercised_summary
if [ "$status" -eq 0 ]; then
    echo "quality-gate: ALL PASS"
else
    echo "quality-gate: FAILED"
fi
exit "$status"
