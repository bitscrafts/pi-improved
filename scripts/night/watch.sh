#!/usr/bin/env bash
# watch.sh — the night's supervisor, with no model in it.
#
# WHY NO MODEL. The first version of this was an agent cron job, and its first firing died with
# `RuntimeError: Gemini HTTP 503` — the job did not inherit the session's model, so a watcher whose whole job is
# mechanical was waiting on a model that was not even the one intended. Every decision below is a file read and a
# command; nothing here needs language.
#
# One bounded step per firing, in this order, stopping at the first that applies:
#   1. past the deadline            -> write the summary, stop
#   2. a worker in flight           -> leave it alone
#   3. the tree is stuck dirty      -> DEFAULT ANSWER: move it aside (never delete) and unblock the queue
#   4. the head row disagrees with  -> DEFAULT ANSWER: repair the spec from the row
#      its spec
#   5. the head row is parked after -> DEFAULT ANSWER: skip it so the queue keeps flowing
#      3 attempts
#   6. the queue is empty           -> night.sh --tick (seed the next tranche, or run Opus)
#
# Env: NIGHT_DIR, NIGHT_UNTIL, NIGHT_STUCK_AFTER (firings a dirty tree may persist, default 2).
set -uo pipefail
cd /projects/pi-improved || exit 1
DIR=${NIGHT_DIR:-_tmp/night}
LOG=$DIR/night.log
STUCK=$DIR/dirty-streak
UNTIL=${NIGHT_UNTIL:-$(date -u +%Y-%m-%d)T10:00:00Z}
STUCK_AFTER=${NIGHT_STUCK_AFTER:-2}
mkdir -p "$DIR"
say(){ printf '%s watch: %s\n' "$(date -u +%H:%M:%SZ)" "$*" | tee -a "$LOG"; }

# 1. the deadline
if [ "$(date -u +%s)" -ge "$(date -u -d "$UNTIL" +%s)" ]; then
  bash scripts/night/night.sh --tick >/dev/null 2>&1 || true     # writes SUMMARY.md and marks done
  say "past $UNTIL — the night is over (summary in $DIR/SUMMARY.md)"
  exit 0
fi

# 2. a worker in flight: the dispatcher owns the queue
if pgrep -f '^bash (.*/)?(scripts/autonomy/cycle|scripts/run-batchC-pii)\.sh' >/dev/null 2>&1; then
  : >"$STUCK"
  say "a worker is in flight — leaving it alone"
  exit 0
fi

# 3. a tree that stays dirty across firings would refuse every row behind it (dirty-tree). Move it aside.
dirty=$(git status --porcelain | wc -l | tr -d ' ')
if [ "$dirty" -gt 0 ]; then
  n=$(( $(cat "$STUCK" 2>/dev/null || echo 0) + 1 )); printf '%s\n' "$n" >"$STUCK"
  if [ "$n" -ge "$STUCK_AFTER" ]; then
    dest=/projects/_trash/night-stuck-$(date -u +%Y%m%dT%H%M%SZ)
    bash scripts/night/clear-stuck-tree.sh "$dest" --apply >/dev/null 2>&1
    : >"$STUCK"
    say "the tree stayed dirty for $n firing(s): moved aside to $dest (nothing deleted) so the queue can proceed"
  else
    say "the tree is dirty ($dirty path(s)); the worker's own cycle gets the next firing before anything is moved"
  fi
  exit 0
fi
: >"$STUCK"

# 3b. rows stuck at three attempts are NOT skipped: skipping would fake closure and let the Opus review run on a
# queue that still holds an unfinished gap. They are named, every firing, so the morning report can say which gap
# held the night.
# and it is RETRIED, not abandoned: the operator's rule is that the next Opus review waits until every gap is
# closed, so a parked row is repaired and handed back to the dispatcher rather than written off. If it parks again,
# the night still has no review — and the morning report says which gap held it.
stuck=$(python3 scripts/autonomy/state.py list 2>/dev/null | awk -F'\t' '$4=="parked" {print $1}' | tr '\n' ' ')
if [ -n "$stuck" ]; then
  for rid in $stuck; do
    python3 scripts/autonomy/state.py repair "$rid" "watch: retrying a parked row so its gap can close" >/dev/null 2>&1 \
      && say "row $rid was parked: repaired and handed back to the dispatcher (the review waits for it)"
  done
  exit 0
fi

# 4/5. the head row
head=$(python3 scripts/autonomy/state.py pick 2>/dev/null | head -1)
case "$head" in
  NONE|'') ;;                                  # empty queue: fall through to the tick
  *)
    id=${head%%$'\t'*}
    if ! bash scripts/night/fix-row.sh --check "$id" >/dev/null 2>&1; then
      out=$(bash scripts/night/fix-row.sh "$id" 2>&1 | tail -2)
      say "row $id disagreed with its spec: repaired from the row — $(printf '%s' "$out" | tr '\n' ' ')"
      exit 0
    fi
    st=$(python3 scripts/autonomy/state.py get "$id" 2>/dev/null | python3 -c 'import json,sys;d=json.load(sys.stdin);print(d.get("status"),d.get("attempts"))' 2>/dev/null)
    case "$st" in
      parked*)
        say "the head row $id is parked ($st) — the queue waits for a repair, not for a skip"
        exit 0 ;;
    esac
    say "row $id is dispatchable — the 5-minute dispatcher has it"
    exit 0 ;;
esac

# 6. empty queue: fill it, or review — DETACHED, because the review step is a 10-12 minute Opus run and a cron
# script that waits for it gets killed mid-run, which would pay for a review nobody can read.
if pgrep -f 'night\.sh --tick' >/dev/null 2>&1; then
  say "the fill/review step is already running — waiting for it"
  exit 0
fi
setsid nohup bash scripts/night/night.sh --tick >>"$LOG" 2>&1 &
say "queue is empty: started the fill/review step in the background (pid $!) — the queue will be refilled from the review"
exit 0
