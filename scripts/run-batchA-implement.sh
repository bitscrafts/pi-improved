#!/usr/bin/env bash
# run-batchA-implement.sh — dispatch the workhorse lane (pi-workhorse harness) for pii specs 70+71+72.
# Two-lane doctrine (hermes/rules/two-lane-pi-analysis-then-workhorse-2026-10-06).
# Model doctrine (hermes/rules/flash-workhorse-full-planner-2026-10-06): worker=flash.
# One spec per harness invocation (run = gate+verify loop for ONE spec); the batch = three runs.
set -uo pipefail

export RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo
export PATH="/usr/local/cargo/bin:/root/.local/bin:$PATH"
export IS_SANDBOX=1
set -a; [ -f /root/.hermes/.env ] && . /root/.hermes/.env; set +a

export ORCH_HOME="/root/.pi-orchestration"
export TIER_ollama_MODEL="glm-5.3-flash:cloud"     # workhorse = flash class
export ORCH_THINKING="off"
export ORCH_MAX_TOOL_ITERATIONS="${ORCH_MAX_TOOL_ITERATIONS:-150}"
export WALL_SECS="${WALL_SECS:-3600}"
SPECS=(
  /projects/pi-improved/specs/70-chunked-decoder-split-safety.md
  /projects/pi-improved/specs/71-bash-render-byte-boundary-truncate.md
  /projects/pi-improved/specs/72-reachability-check-measures-real-callers.md
)

echo "== workhorse lane: batch A (${#SPECS[@]} specs) worker=$TIER_ollama_MODEL"
overall=0
for SPEC in "${SPECS[@]}"; do
  echo "== spec: $(basename "$SPEC")"
  /root/.pi-orchestration/bin/pi-workhorse.sh run "$SPEC" /projects/pi-improved
  rc=$?
  echo "== spec $(basename "$SPEC") harness rc=$rc"
  [ "$rc" -ne 0 ] && overall=$rc
done
echo "BATCH-OVERALL=$overall (0=clean; 3 gate-fail; 4 exit-criteria; 5 spec-wrong; 6 nothing; 7 committed=UNWIND)"
exit "$overall"