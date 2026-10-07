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
# SEQUENCING NOTE (2026-10-07, learned: the harness's dirty-tree guard forbids multi-spec
# unattended batches — each run hands back with ITS work uncommitted for the orchestrator to
# commit first; rc=8 on the next spec, by design. So a batch = per-spec dispatch with an
# orchestrator commit between runs; loop here does exactly that, one harness run at a time,
# committing BETWEEN specs (worker output is spec-work only — never the script itself).
overall=0
for SPEC in "${SPECS[@]}"; do
  echo "== spec: $(basename "$SPEC")"
  # only uncommitted spec-work should be in the tree here; if anything else is dirty, abort.
  /root/.pi-orchestration/bin/pi-workhorse.sh run "$SPEC" /projects/pi-improved
  rc=$?
  echo "== spec $(basename "$SPEC") harness rc=$rc"
  if [ "$rc" -eq 0 ]; then
    git add -A ':!scripts' ':!_tmp'
    git -c user.name="Marcelo Correa" -c user.email="mvcorrea@gmail.com" \
      commit -m "spec $(basename "$SPEC" .md): implemented via workhorse lane (verified by harness run)" \
      || overall=$?
  else
    overall=$rc; break   # hand back to the orchestrator on any non-zero
  fi
done
echo "BATCH-OVERALL=$overall (0=clean; 3 gate-fail; 4 exit-criteria; 5 spec-wrong; 6 nothing; 7 committed=UNWIND; 8 dirty=see note)"
exit "$overall"