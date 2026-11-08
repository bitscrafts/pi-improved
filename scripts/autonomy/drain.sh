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
# PII_DRAIN_ROOT is a seam for tests only (a throwaway repo holding a copy of scripts/autonomy and a stub cycle.sh); the cron never sets it.
cd "${PII_DRAIN_ROOT:-/projects/pi-improved}" || exit 1
# The tree hold (2026-11-01): an interactive turn can be editing this tree with no lock of its own, so a dispatcher
# that only checks the cycle's flock cannot know. Passing --from-cron is what makes this a BACKGROUND caller: it
# yields and exits 0, which reduces its rights and can never gain any.
bash scripts/autonomy/hold.sh check >/dev/null 2>&1
if [ $? -eq 3 ]; then
  bash scripts/autonomy/hold.sh check | sed 's/^hold: /drain: /'
  exit 0
fi
. scripts/autonomy/env.sh 2>/dev/null
# THE TWO CHECKS THAT LET THIS CRON STAY ON AND STILL NEVER COLLIDE (operator, 2026-11-04: "cron must
# always be on!" and "we need some check before the cronjob start or peek anything from the queue").
#
# The hold is a DECLARATION and only works if a person declares it. These two are MEASUREMENTS, so they
# hold whoever forgot:
#  (a) anything already running — a lane worker or a cycle — means this firing has nothing to add;
#  (b) uncommitted work at the start of a firing belongs to someone else. Measured 2026-11-04 14:52Z:
#      this dispatcher dispatched row 384 into the tree holding row 380's WIP, then 380's gate failed
#      on 384's test while reporting on 380. A clean tree at the start of a firing is the precondition
#      for "the diff the gate measures is the diff this row wrote".
RUNNING_NOW=$(pgrep -af 'run-batchC-(pii|claude)|scripts/autonomy/cycle\.sh' 2>/dev/null | grep -v "^$$ " | head -3)
if [ -n "$RUNNING_NOW" ]; then
  echo "drain: a worker or cycle is already running — nothing started"
  printf '%s\n' "$RUNNING_NOW" | sed 's/^/drain:   /'
  exit 0
fi
DIRTY_NOW=$(git status --porcelain 2>/dev/null | head -5)
if [ -n "$DIRTY_NOW" ]; then
  # A DIRTY TREE IS NOT ENOUGH TO STOP: ASK WHOSE IT IS (measured 2026-11-05, the night that produced
  # nothing: row 391's worker finished at 03:11, the tree kept its 25 paths, this guard refused every
  # later firing, and 6.5 hours passed with 78 firings and no commit. A worker never commits — the
  # cycle does — so a tree holding a finished row's work must be RESUMED to its gate, not frozen.)
  OWN=$(bash scripts/autonomy/leftover.sh owner 2>/dev/null); ORC=$?
  if [ "$ORC" = 0 ]; then
    ROW=${OWN#OWNER }
    echo "drain: the tree holds row $ROW's unfinished work — resuming THAT row to its gate (no new dispatch)"
    printf '%s\n' "$DIRTY_NOW" | sed 's/^/drain:   /'
    export PII_FORCE_ITEM="$ROW"
    export ALLOW_DIRTY=1
  else
    echo "drain: the tree has uncommitted work that no queue row owns ($OWN) — not dispatching over it"
    printf '%s\n' "$DIRTY_NOW" | sed 's/^/drain:   /'
    # A SILENT REFUSAL IS HOW SIX AND A HALF HOURS PASSED (78 firings, nothing said). An unowned tree is
    # still nobody's to touch — but the operator must be told, once an hour rather than once per firing.
    STAMP=_tmp/autonomy/foreign-alarm
    LAST=$(cat "$STAMP" 2>/dev/null || echo 0)
    NOWS=$(date +%s)
    if [ $((NOWS - LAST)) -ge 3600 ]; then
      bash scripts/autonomy/control.sh ask "a dirty tree no queue row owns ($OWN) is blocking the dispatcher: $(printf '%s' "$DIRTY_NOW" | tr '\n' ' ')" >/dev/null 2>&1 || true
      printf '%s\n' "$NOWS" > "$STAMP"
      echo "drain: ALARM raised — this tree blocks every firing until it is committed or moved aside"
    fi
    exit 0
  fi
fi
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
  # THE HOLD IS RE-CHECKED BEFORE EVERY CYCLE, not only at the top of the firing (measured 2026-11-06: a hold taken after a
  # firing began was ignored for up to 8 cycles; one firing re-dispatched the same row every ~3 minutes for ~25 minutes). It is a
  # yield, never a pause: the cron stays on, this firing ends, the next one asks again.
  if ! bash scripts/autonomy/hold.sh check >/dev/null 2>&1; then
    bash scripts/autonomy/hold.sh check | sed 's/^hold: /drain: /'
    echo "drain: the tree is held — stopping before cycle $i, nothing started, nothing spent"
    break
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
