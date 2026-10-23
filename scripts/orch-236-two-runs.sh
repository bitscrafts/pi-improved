#!/usr/bin/env bash
# Spec 236 R5: two consecutive `cargo xtask fresh-gate` runs at one HEAD must both report PASS.
# Exit 0 only when both runs print `fresh-gate: PASS`; exit 1 names the run that did not.
set -u
cd "$(dirname "$0")/.." || exit 2
head_before=$(git rev-parse HEAD) || exit 2
for n in 1 2; do
    out=$(cargo run -q -p xtask -- fresh-gate 2>&1)
    rc=$?
    printf '%s\n' "$out" | tail -n 4
    if [ "$rc" -ne 0 ] || ! printf '%s\n' "$out" | grep -qx 'fresh-gate: PASS'; then
        echo "two-runs: FAIL, run $n did not report PASS (rc=$rc)"
        exit 1
    fi
    echo "two-runs: run $n PASS"
done
if [ "$(git rev-parse HEAD)" != "$head_before" ]; then
    echo "two-runs: FAIL, HEAD moved during the pair"
    exit 1
fi
echo "two-runs: PASS (both runs at $head_before)"
