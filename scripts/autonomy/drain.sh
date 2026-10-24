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
# THE WORKER IS DECLARED HERE, so the scheduled dispatcher is self-sufficient (user, 2026-10-19: "it should be automatic").
# claude + haiku is the implementation workhorse; a closed window makes cycle.sh return OUTCOME=window-closed and the NEXT
# firing of this dispatcher resumes automatically once the reset has passed — no hand-made one-shot needed.
export PII_WORKER="${PII_WORKER:-pii}"
export CLAUDE_MODEL="${CLAUDE_MODEL:-claude-haiku-5-5}"
MAX=${MAX_CYCLES:-8}
QUIET=${PII_DRAIN_QUIET:-0}
ST=_tmp/autonomy/control
say() { [ "$QUIET" = 1 ] || echo "$@"; }

if ! grep -q RUNNING "$ST" 2>/dev/null; then
  echo "drain: control is not RUNNING — $(head -1 "$ST" 2>/dev/null || echo 'no control file')"
  exit 0
fi
# WAIT for the lock instead of bailing on it (measured 2026-10-19: a */30 dispatcher that bailed instantly left the queue
# idle for up to 30 minutes whenever a cycle from another source owned the lock; the queue's rule is that items drain).
if ! flock -w "${PII_LOCK_WAIT:-120}" -E 75 _tmp/autonomy/lock true 2>/dev/null; then
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
  SPEC=$(printf '%s' "$P" | cut -f2)
  # A spec that says it is superseded or not queued must never be dispatched again. Measured 2026-10-19: 144 (the judge node)
  # carries "RESEARCH - NOT QUEUED FOR IMPLEMENTATION" and "SUPERSEDED ... do not implement", and the queue re-dispatched it
  # twice, spending a worker turn to be told by the file itself that it should not be built.
  if [ -f "specs/$SPEC" ] && grep -qiE 'SUPERSEDED|NOT QUEUED FOR IMPLEMENTATION' "specs/$SPEC"; then
    reason=$(grep -m1 -oiE 'SUPERSEDED|NOT QUEUED FOR IMPLEMENTATION' "specs/$SPEC")
    python3 scripts/autonomy/state.py set "$ITEM" status=skipped "last=spec says $reason - never dispatch it again" >/dev/null 2>&1
    echo "drain: $ITEM retired without dispatch — $SPEC says $reason"
    continue
  fi
  say "drain: [$i] $ITEM"
  OUT=$(bash scripts/autonomy/cycle.sh 2>&1 | tail -1)
  say "        $OUT"
  case "$OUT" in
    *OUTCOME=done*|*OUTCOME=done-satisfied*) done_n=$((done_n + 1)) ;;
    *OUTCOME=queue-empty*|*OUTCOME=paused*|*OUTCOME=locked*) break ;;
    *OUTCOME=plan-error*) echo "drain: cycle refused — $OUT"; exit 1 ;;
  esac
done

# Keep pii's memory in step with the record: idempotent, so a pass with nothing new writes nothing.

echo "drain: closed $done_n row(s) this firing"
if [ "$(python3 scripts/autonomy/state.py pick 2>/dev/null | head -1)" = "" ]; then
  BRIEF=$(bash scripts/autonomy/brief-path.sh)
  bash scripts/autonomy/queue-brief.sh > "$BRIEF" 2>/dev/null || true
  echo "drain: QUEUE EMPTY — brief written to $BRIEF"
fi
