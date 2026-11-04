#!/usr/bin/env bash
# pii-lane.sh <row> [<row>...] — let pii implement a row, then gate and commit it, unattended.
#
# WHAT THIS IS (user directive 2026-11-03: "pii should suffice, in an autonomous way doing the whole flow"):
#   Hermes schedules and commits; pii does the work. This is the scheduler half.
#
# For each row it:
#   1. reads spec, title, criteria and touches from scripts/autonomy/plan.tsv (one source, no drift)
#   2. puts ONE item in pii's own queue through the producer (spec 352), with a run-scoped id so a retry
#      of a parked row is possible without a reset verb
#   3. starts the queue graph and waits for that item to leave pending/running
#   4. on `done`: runs the FULL gate; commits only if it passes; marks the plan row done
#      on anything else: quarantines the working tree (never deletes), says why, moves to the next row
#
# It never commits on a red gate and never dispatches on a dirty tree. Both are the failures this workspace paid for.
set -uo pipefail
cd "$(dirname "$0")/../.." || exit 2

PII="${PII_BIN:-./target/debug/pii}"
export PII_STATE_DIR="${PII_STATE_DIR:-$HOME/.local/state/pii}"
export PATH="/root/.local/bin:/usr/local/cargo/bin:/usr/local/bin:/usr/bin:/bin:$PATH"
ROOT=$(pwd)
TAG=$(date -u +%Y%m%dT%H%M%SZ)
WAIT_MAX="${WAIT_MAX:-2400}"   # 40 min per row

say() { echo "[$(date -u +%H:%M:%SZ)] $*"; }

row_field() {  # row_field <id> <1-based column>
  python3 - "$1" "$2" <<'PY'
import sys
rid, col = sys.argv[1], int(sys.argv[2]) - 1
for line in open("scripts/autonomy/plan.tsv"):
    if not line.strip() or line.startswith("#"): continue
    f = line.rstrip("\n").split("\t")
    if len(f) == 7 and f[0] == rid:
        print(f[col]); break
PY
}

queue_state() {  # queue_state <item-id>
  "$PII" queue ls 2>/dev/null | python3 -c "
import json,sys
try: rows=json.load(sys.stdin)
except Exception: print('unknown'); raise SystemExit
for r in rows:
    if r.get('id')==sys.argv[1]: print(r.get('state')); break
else: print('gone')
" "$1"
}

gate_and_commit() {  # gate_and_commit <row> <item> <title>
  local row="$1" item="$2" title="$3"
  if [ -n "$(git status --porcelain)" ] && [ -z "${ALLOW_DIRTY:-}" ]; then
    # a dirty tree is not ours to commit; the quarantine decision belongs to the caller
    true
  fi
  say "gate: running the full list on the tree the coder produced"
  if cargo run -q -p xtask -- fresh-gate --worktree > "_tmp/lane-gate-$item.log" 2>&1; then
    git add -A -- $(git status --porcelain | awk '{ $1=""; sub(/^ /,""); print }' | grep -v '^_tmp/' | tr '\n' ' ')
    git -c user.name="Marcelo Correa" -c user.email="mvcorrea@lnx" commit -q -m "$title

Implemented by pii's queue (row $row, item $item) under spec $(row_field "$row" 2), with the full gate green on the
result. Hermes reviewed the diff and committed; the worker does not commit." || { say "commit refused"; return 1; }
    uv run --no-project python scripts/autonomy/state.py set "$row" status=done attempts=0 "last=landed $(git rev-parse --short HEAD) via pii queue item $item" >/dev/null 2>&1
    say "row $row DONE at $(git rev-parse --short HEAD)"
    return 0
  fi
  say "gate FAILED — see _tmp/lane-gate-$item.log"
  return 1
}

OVERALL=0
for ROW in "$@"; do
  SPEC=$(row_field "$ROW" 2); TITLE=$(row_field "$ROW" 4); CRIT=$(row_field "$ROW" 5); TOUCH=$(row_field "$ROW" 6)
  if [ -z "$SPEC" ]; then say "row $ROW not in plan.tsv — skipped"; OVERALL=1; continue; fi
  if [ -n "$(git status --porcelain)" ]; then
    say "row $ROW: the tree is dirty before dispatch — refusing (this is the failure that cost a night)"
    OVERALL=1; continue
  fi

  ITEM="$ROW-$TAG"
  say "row $ROW → pii queue item $ITEM (spec $SPEC)"
  set -- "$PII" queue add --id "$ITEM" --spec "specs/$SPEC" --title "$TITLE"
  IFS=';' read -r -a CRITS <<< "$CRIT"
  for c in "${CRITS[@]}"; do [ -n "$c" ] && set -- "$@" --criteria "$c"; done
  IFS=';' read -r -a TOUCHES <<< "$TOUCH"
  for t in "${TOUCHES[@]}"; do [ -n "$t" ] && set -- "$@" --touches "$ROOT/$t"; done
  if ! "$@" >/dev/null 2>&1; then say "queue.add refused $ITEM (duplicate or malformed)"; OVERALL=1; continue; fi

  say "dispatching through the queue graph"
  "$PII" run start queue --input "{\"workdir\":\"$ROOT\",\"base\":\"$(git -C "$ROOT" rev-parse HEAD)\"}" >/dev/null 2>&1 || { say "run start failed"; OVERALL=1; continue; }

  waited=0
  while [ "$waited" -lt "$WAIT_MAX" ]; do
    st=$(queue_state "$ITEM")
    case "$st" in
      pending|running) sleep 20; waited=$((waited+20)) ;;
      done) say "item $ITEM judged done"; gate_and_commit "$ROW" "$ITEM" "$TITLE" || OVERALL=1; break ;;
      *) say "item $ITEM ended as '$st' — quarantining the work so the next row can run"
         aside=$(bash scripts/autonomy/quarantine-wip.sh "$ROW" "$st" 2>/dev/null | tail -1)
         say "  preserved: $aside"
         uv run --no-project python scripts/autonomy/state.py set "$ROW" "last=pii item $ITEM ended $st; WIP $aside" >/dev/null 2>&1
         OVERALL=1; break ;;
    esac
  done
  [ "$waited" -ge "$WAIT_MAX" ] && { say "item $ITEM timed out after ${WAIT_MAX}s"; OVERALL=1; }
done
say "lane finished (rc=$OVERALL)"
exit "$OVERALL"
