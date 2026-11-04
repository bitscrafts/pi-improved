#!/usr/bin/env bash
# run-batch-rows.sh <row>... — implement SEVERAL rows in ONE tree, then gate once.
#
# Why this exists (2026-11-04, operator: "just one row! it took too much time!"): a row costs a worker
# turn (~4 min) plus a full gate (~12 min), and the two serialise because the worker needs a clean tree.
# That is ~16 minutes per row and it was the real bottleneck. A batch pays one gate for several rows;
# the commits stay separate, so a red gate still names the row that reddened it.
#
# Rules it keeps:
#   · every row's criteria are EXECUTED before its worker runs, so a row that is already green is
#     reported and skipped rather than paid for twice;
#   · the worker runs through the existing lane script, never an inline command;
#   · ALLOW_DIRTY is set because rows in a batch deliberately share one tree — the rows are ordered so
#     a later row builds on the earlier one (the manifest lists the set the merge reshapes);
#   · the batch stops at the first row whose worker fails, leaving the tree as it is for inspection.
set -uo pipefail
cd /projects/pi-improved || exit 1
export PATH=/root/.local/bin:/usr/local/cargo/bin:$PATH
CRIT="uv run --no-project python scripts/autonomy/row-run-criteria.py"
[ $# -ge 1 ] || { echo "usage: run-batch-rows.sh <row>..."; exit 2; }

for row in "$@"; do
  echo "════ row $row"
  before=$($CRIT "$row" 2>&1 | tail -1)
  echo "   before: ${before#RESULT: }"
  case "$before" in
    *"0 failed"*) echo "   SKIP $row — its criteria are already green; nothing to implement"; continue ;;
  esac
  ALLOW_DIRTY=1 bash scripts/run-batchC-pii.sh "$row" > "_tmp/batch-$row.log" 2>&1
  rc=$?
  after=$($CRIT "$row" 2>&1 | tail -1)
  echo "   after : ${after#RESULT: } (worker rc=$rc)"
  if [ "$rc" -ne 0 ]; then
    echo "   STOP — the worker for $row failed; see _tmp/batch-$row.log"
    exit 1
  fi
done

echo "════ batch finished · tree paths: $(git status --porcelain | wc -l)"
git status --porcelain | head -20
echo "════ one gate for the whole batch:"
export FRESH_GATE_TARGET=/projects/pi-improved/_tmp/gate-target
unset GATE_STEPS CARGO_TARGET_DIR PII_STATE_DIR PII_RUNTIME_DIR GATE_WRITE_DIRS
cargo run -q -p xtask -- fresh-gate --worktree > /tmp/gate-batch.log 2>&1
echo "gate rc=$?  passes=$(grep -cE '^PASS:' /tmp/gate-batch.log)"
grep -E '^FAIL:' /tmp/gate-batch.log | head -5
