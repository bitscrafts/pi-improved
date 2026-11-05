#!/usr/bin/env bash
# Resume ONE row at verify: its worker already finished and left the work in the tree; only the criteria, the gate and
# the commit remain. No worker is dispatched (the cycle's `worker-ok` phase marker skips it). Holds the tree for the
# duration and releases on every exit. Usage: resume-row-at-verify.sh <row>
set -uo pipefail
ROW=${1:?row}
cd /projects/pi-improved || exit 1
export PATH=/root/.local/bin:/usr/local/cargo/bin:/usr/local/bin:/usr/bin:/bin:${PATH:-}
# Take the CYCLE lock the same way the cron's cycle does (flock -E 75): a cron cycle in flight would not yield it,
# and a started-then-locked resume does nothing but print OUTCOME=locked (measured twice, 2026-11-05).
exec 9>/projects/pi-improved/_tmp/autonomy/lock
flock -n 9 || { echo "STOPPED: the cycle lock is held (a cron cycle is in flight); run this again after it ends"; exit 75; }
OWNER="resume-$ROW-$$"
bash scripts/autonomy/hold.sh take "$OWNER" 14400 100 || { echo "STOPPED: could not take the tree hold"; exit 8; }
trap 'bash scripts/autonomy/hold.sh release "$OWNER" >/dev/null 2>&1; echo "RELEASED : $OWNER"' EXIT
echo "row $ROW before: $(python3 scripts/autonomy/state.py get "$ROW" | tr -d '\n ' | cut -c1-170)"
echo "tree: $(git status --porcelain | wc -l) changed paths"
python3 scripts/autonomy/state.py set "$ROW" status=pending last=worker-ok >/dev/null
OUT=$(PII_FORCE_ITEM="$ROW" bash scripts/autonomy/cycle.sh 2>&1 | tail -8)
echo "$OUT" | sed 's/^/   /' | cut -c1-190
echo "row $ROW after: $(python3 scripts/autonomy/state.py get "$ROW" | tr -d '\n ' | cut -c1-190)"
echo "HEAD $(git log --oneline -1 | cut -c1-80)   dirty $(git status --porcelain | wc -l)"
