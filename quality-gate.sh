#!/usr/bin/env bash
# Quality gate: fmt, clippy, test, coverage floor and the xtask repo checks.
# Exit 0 only if every step passes. The steps live in gate-steps.txt (the one list, read by the
# local run and the fresh-gate export alike); scripts/gate-lib.sh runs each with a heartbeat.
# Two tiers (spec 171):
#   --routine (the default)  runs the steps in THIS repo against the warm _tmp/gate-target: minutes. Per item.
#   --fresh                  the CI-equivalent fresh-tree export (cargo xtask fresh-gate --worktree; recompiles
#                            the workspace, ~45 min). The release boundary: run it before a deploy, and the
#                            deploy card quotes it as "mode=fresh".

set -uo pipefail

# The step list is selectable: routine items use gate-steps-quick.txt (fmt, clippy, build — minutes); the
# commit and release boundaries use the full gate-steps.txt (workspace tests, coverage, doctests, scans).
: "${GATE_STEPS:=gate-steps.txt}"
export GATE_STEPS

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

mode=routine
case "${1:-}" in
    --fresh) mode=fresh ;;
    --routine | '') mode=routine ;;
    *)
        echo "usage: quality-gate.sh [--routine|--fresh|--summary]" >&2
        exit 2
        ;;
esac

if [ "$mode" = "fresh" ]; then
    echo "quality-gate: mode=fresh (fresh-tree export, the release boundary)"
    CARGO_TARGET_DIR="${FRESH_GATE_TARGET:-/projects/pi-improved/_tmp/gate-target}" \
        FRESH_GATE_MIN_FREE_MB="${FRESH_GATE_MIN_FREE_MB:-12000}" \
        cargo run -q -p xtask -- fresh-gate --worktree
    exit $?
fi
echo "quality-gate: mode=routine (repo tree, warm cache)"

status=0
steps=0
while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in '' | '#'*) continue ;; esac
    run_step "${line%%|*}" "${line#*|}" || status=1
    steps=$((steps + 1))
done <"$GATE_STEPS"

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
