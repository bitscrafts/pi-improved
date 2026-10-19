#!/usr/bin/env bash
# drain.sh — THE dispatcher. One cron entry, no model, no judgement: drain the queue as far as it will go in one firing.
#
# Created 2026-10-19 after three overlapping */30 jobs (a tick, a single-cycle runner and an agent that drained) spent the
# day contending for the same lock: two of every three invocations logged OUTCOME=locked, and the queue could sit with a
# pending head while the logs looked busy. One owner is the fix, and it should not need a model call to decide "run the
# next row".
#
# Behaviour, in order:
#   · control file not RUNNING        → say so, exit 0 (nothing spent)
#   · lock held by someone else       → say so, exit 0 (do not queue up behind it)
#   · pick returns nothing            → write the queue-end brief, exit 0
#   · pick refuses (plan-error)       → say so, exit 1 (a refusal is a fault, not an empty queue)
#   · otherwise run cycle.sh, up to MAX_CYCLES, stopping early on queue-empty / paused / locked
#
# Env: MAX_CYCLES (default 8), PII_DRAIN_QUIET=1 to print only the summary.
set -uo pipefail
cd /projects/pi-improved || exit 1
. scripts/autonomy/env.sh 2>/dev/null
MAX=${MAX_CYCLES:-8}
QUIET=${PII_DRAIN_QUIET:-0}
ST=_tmp/autonomy/control
say() { [ "$QUIET" = 1 ] || echo "$@"; }

if ! grep -q RUNNING "$ST" 2>/dev/null; then
  echo "drain: control is not RUNNING — $(head -1 "$ST" 2>/dev/null || echo 'no control file')"
  exit 0
fi
if ! flock -n -E 75 _tmp/autonomy/lock true 2>/dev/null; then
  echo "drain: another cycle holds the lock — nothing started"
  exit 0
fi

done_n=0
for i in $(seq 1 "$MAX"); do
  P=$(python3 scripts/autonomy/state.py pick 2>/dev/null | head -1)
  case "$P" in
    plan-error*) echo "drain: pick REFUSED — $P"; exit 1 ;;
    "")          break ;;
  esac
  ITEM=$(printf '%s' "$P" | cut -f1)
  say "drain: [$i] $ITEM"
  OUT=$(bash scripts/autonomy/cycle.sh 2>&1 | tail -1)
  say "        $OUT"
  case "$OUT" in
    *OUTCOME=done*|*OUTCOME=done-satisfied*) done_n=$((done_n + 1)) ;;
    *OUTCOME=queue-empty*|*OUTCOME=paused*|*OUTCOME=locked*) break ;;
    *OUTCOME=plan-error*) echo "drain: cycle refused — $OUT"; exit 1 ;;
  esac
done

echo "drain: closed $done_n row(s) this firing"
if [ "$(python3 scripts/autonomy/state.py pick 2>/dev/null | head -1)" = "" ]; then
  BRIEF="_docs/05-operations/QUEUE-BRIEF-$(date -u +%Y-%m-%dT%H%M%SZ).md"
  mkdir -p _docs/05-operations
  bash scripts/autonomy/queue-brief.sh > "$BRIEF" 2>/dev/null || true
  echo "drain: QUEUE EMPTY — brief written to $BRIEF"
fi
