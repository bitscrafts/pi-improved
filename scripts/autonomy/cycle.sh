#!/usr/bin/env bash
# cycle.sh — ONE unit of autonomous work, from probe to commit-or-park.
#
# What it does, in order, and why in that order:
#   0  take the lock            — two cycles would collide on the tree and on the disk
#   1  probe the account        — never start a long run on a closed window (exit 3, nothing spent)
#   2  pick the top pending item— the plan is a file, not a conversation
#   3  dispatch the worker      — via the project's own script (session id is recorded there)
#   4  run the item's verify    — the item's own commands, from the plan
#   5  the gate                 — fresh-gate when disk allows; a green verify is not a green gate
#   6  commit, or park          — three strikes parks the item and the queue moves on
#
# It is deterministic ON PURPOSE: the reasoning (authoring a spec, judging a mutation, a live proof)
# belongs to the agent that calls it; the mechanics belong here, where they can be tested.
#
# env: PLAN, GATE=1|0 (default 1), DRY=1 prints the plan and stops before dispatching.
set -uo pipefail
ROOT=/projects/pi-improved; cd "$ROOT" || exit 1
STATEDIR=$ROOT/_tmp/autonomy; LOGD=$STATEDIR/logs
mkdir -p "$LOGD" _tmp/claude-batchC
CID=$(date -u +%Y%m%dT%H%M%SZ)
LOG=$LOGD/cycle-$CID.log
say(){ printf '[%s] %s\n' "$(date -u +%H:%M:%SZ)" "$*"; }
say "=== cycle $CID"
exec 9>"$STATEDIR/lock"
if ! flock -n 9; then say "another cycle holds the lock — nothing done"; echo "OUTCOME=locked"; exit 5; fi

# 1) the window
PROBE=$(bash scripts/autonomy/probe.sh 2>&1); PRC=$?
echo "$PROBE" >>"$LOG"
case $PRC in
  0) say "probe: $(printf '%s' "$PROBE" | head -1)";;
  3) say "QUOTA CLOSED — nothing spent, the item was NOT advanced"; say "$PROBE"; echo "OUTCOME=window-closed"; exit 3;;
  *) say "PROBE UNDECIDED rc=$PRC — stopping rather than guessing about the window"; say "$PROBE"; echo "OUTCOME=probe-error"; exit 4;;
esac

# 2) the plan
PICK=$(python3 scripts/autonomy/state.py pick)
if [ "$PICK" = "NONE" ]; then say "queue empty — nothing pending"; echo "OUTCOME=queue-empty"; exit 0; fi
IFS=$'\t' read -r ITEM SPEC BRIEF TITLE VERIFY PATHS GATE ATTEMPTS RSID <<<"$PICK"
say "item=$ITEM spec=$SPEC attempts=$ATTEMPTS title=$TITLE"
if [ ! -f "specs/$SPEC" ]; then
  say "NO SPEC at specs/$SPEC — the agent must AUTHOR it before this cycle can spend anything"
  echo "OUTCOME=needs-spec ITEM=$ITEM TITLE=$TITLE"; exit 6
fi
if [ -n "$BRIEF" ] && [ "$BRIEF" != "-" ] && [ ! -f "$BRIEF" ]; then
  say "NO BRIEF at $BRIEF"; echo "OUTCOME=needs-brief ITEM=$ITEM"; exit 6
fi
[ -n "${DRY:-}" ] && { say "DRY=1: would dispatch spec=$SPEC brief=$BRIEF resume=${RSID:-none}"; echo "OUTCOME=dry"; exit 0; }

# 3) the worker — resume-first, the session id comes from the state, not from memory
DIRTY=$(git status --porcelain | wc -l)
say "tree: $DIRTY changed paths before dispatch"
if [ "$DIRTY" -ne 0 ] && [ -z "$RSID" ]; then
  say "DIRTY TREE and no session to resume — refusing to dispatch onto someone else's work"
  git status --porcelain | head -10 >>"$LOG"; echo "OUTCOME=dirty-tree"; exit 7
fi
ARGS=(); [ -n "$RSID" ] && ARGS=(--resume "$RSID")
BRIEFARG=(); [ -n "$BRIEF" ] && [ "$BRIEF" != "-" ] && BRIEFARG=("$BRIEF")
( ALLOW_DIRTY=1 bash scripts/run-batchC-claude.sh "$SPEC" "${ARGS[@]}" "${BRIEFARG[@]}" ) >>"$LOG" 2>&1
WRC=$?
NEWSID=$(awk '{print $1}' _tmp/claude-batchC/session-"$SPEC"-*.txt 2>/dev/null | tail -1)
say "worker rc=$WRC session=${NEWSID:-none}"
python3 scripts/autonomy/state.py set "$ITEM" status=needs-review "session=${NEWSID:-}" last=worker-rc=$WRC
[ $WRC -ne 0 ] && { say "worker did not finish cleanly — leaving the WIP for the next cycle to resume"; echo "OUTCOME=worker-rc-$WRC ITEM=$ITEM SESSION=${NEWSID:-}"; exit 8; }

# 4) the item's own criteria — run from the plan, on the tree the worker left
VFAIL=""
if [ -n "$VERIFY" ] && [ "$VERIFY" != "-" ]; then
  IFS=';' read -ra CMDS <<<"$VERIFY"
  for c in "${CMDS[@]}"; do
    say "verify: $c"
    if bash -lc "$c" >>"$LOG" 2>&1; then say "  rc=0"; else say "  rc!=0"; VFAIL="$c"; fi
  done
fi
if [ -n "$VFAIL" ]; then
  python3 scripts/autonomy/state.py set "$ITEM" status=needs-review attempts=$((ATTEMPTS+1)) "last=verify-failed"
  [ $((ATTEMPTS+1)) -ge 3 ] && python3 scripts/autonomy/state.py set "$ITEM" status=parked "last=3-strikes"
  say "VERIFY FAILED: $VFAIL"; echo "OUTCOME=verify-failed ITEM=$ITEM CMD=$VFAIL"; exit 9
fi

# 5) the gate — a green verify is not a green gate
if [ "${GATE:-1}" = "1" ]; then
  FREE=$(df -m --output=avail /projects | tail -1 | tr -d ' ')
  if [ "$FREE" -lt 20000 ]; then say "gate SKIPPED: only ${FREE}M free (needs 20000M) — recorded, not pretended"; else
    say "fresh-gate (this is the project's truth, not the worker's)"
    if ( export PATH="/usr/local/cargo/bin:$PATH"; FRESH_GATE_MIN_FREE_MB=12000 timeout 1780 cargo run -q -p xtask -- fresh-gate ) >>"$LOG" 2>&1
    then say "fresh-gate PASS"; else
      python3 scripts/autonomy/state.py set "$ITEM" status=needs-review attempts=$((ATTEMPTS+1)) last=gate-failed
      say "fresh-gate FAILED — see $LOG"; echo "OUTCOME=gate-failed ITEM=$ITEM"; exit 10
    fi
  fi
fi

# 6) commit — explicit paths only, neutral identity, no tool ever named
if [ -n "$PATHS" ] && [ "$PATHS" != "-" ]; then
  IFS=';' read -ra P <<<"$PATHS"
  git add -- "${P[@]}" || { say "git add failed"; echo "OUTCOME=add-failed ITEM=$ITEM"; exit 11; }
  if git diff --cached --quiet; then say "nothing staged — the worker changed nothing"; echo "OUTCOME=no-change ITEM=$ITEM"; exit 12; fi
  git -c user.name="Marcelo Correa" -c user.email="mvcorrea@gmail.com" commit -q \
    -m "$SPEC: automated pass — $TITLE (spec + criteria in specs/$SPEC; gate green)" || { say "commit failed"; exit 11; }
  say "committed: $(git log --oneline -1)"
fi
python3 scripts/autonomy/state.py set "$ITEM" status=done attempts=$((ATTEMPTS+1)) "last=committed $(git rev-parse --short HEAD)"
say "ITEM $ITEM DONE"; echo "OUTCOME=done ITEM=$ITEM HEAD=$(git rev-parse --short HEAD)"
