#!/usr/bin/env bash
# incident-fix.sh <fingerprint> — spec 151: an open incident reaches a PROVEN fix on a branch, never on master
# by assumption. In order:
#   1 read the incident (spec 150's incidents.jsonl)        4 worker, in the worktree, on incident/<fp>
#   2 budget: one dispatch per fingerprint per local day     5 the proof, run HERE: test on base (must be RED),
#     plus a recorded cost ceiling; a refusal is recorded        test on branch (must be GREEN), gate on branch
#   3 brief + worktree from the recorded base revision       6 all legs green -> fast-forward master; else PARK
# The incident record may carry `test` (the proving command, run from a worktree root) and `base` (a revision;
# default: master's tip). No `test` = nothing to prove = parked. Nothing here writes under specs/.
# env: PII_INCIDENT_WORKER (cmd, runs in the worktree; PII_INCIDENT_BRIEF/_FP/_COST_FILE are set),
#      PII_INCIDENT_GATE (default: fresh-gate), PII_INCIDENT_MERGE=1|0 (default 1), PII_INCIDENT_COST_CEILING
#      (USD, default 5), PII_INCIDENT_DAY (override the local day), PII_INCIDENTS, PII_INCIDENT_STATE.
# stdout: the legs line and `OUTCOME=merged|proven|parked|refused|error FP=.. REASON=..`; exit 0 only for merged/proven.
. "$(dirname "$(readlink -f "$0")")/env.sh"
set -uo pipefail
FP="${1:-}"
[ -n "$FP" ] || { echo "usage: incident-fix.sh <fingerprint>"; exit 2; }
case "$FP" in *[!A-Za-z0-9._-]*|.*) echo "OUTCOME=error FP=$FP REASON=unsafe-fingerprint"; exit 2;; esac
REPO="${PII_REPO:-/projects/pi-improved}"
cd "$REPO" || exit 2
SD="${PII_INCIDENT_STATE:-$REPO/_tmp/autonomy}"
INC="${PII_INCIDENTS:-$SD/incidents.jsonl}"
[ -f "$INC" ] || INC="$SD/logs/incidents.jsonl"
DIR="$SD/incidents/$FP"; BRIEFS="$SD/briefs"
mkdir -p "$DIR" "$BRIEFS" || exit 2
DAY="${PII_INCIDENT_DAY:-$(date +%F)}"
CEILING="${PII_INCIDENT_COST_CEILING:-5}"
GATE_CMD="${PII_INCIDENT_GATE:-cargo run -q -p xtask -- fresh-gate --worktree}"
ID=(-c user.name="Marcelo Correa" -c user.email="mvcorrea@gmail.com")
say(){ printf '%s\n' "$*"; }

# 1) the incident: the LAST line of this fingerprint (a count raise is a newer line)
FIELDS=$(python3 - "$INC" "$FP" <<'PY'
import json,sys
row=None
try:
    for l in open(sys.argv[1]):
        try: d=json.loads(l)
        except Exception: continue
        if d.get("fingerprint")==sys.argv[2]: row=d
except OSError: pass
if row is None: sys.exit(3)
for k in ("first_seen","count","evidence","test","base","source"):
    print(str(row.get(k,"")).replace("\n"," "))
PY
) || { say "OUTCOME=error FP=$FP REASON=no-such-incident"; exit 2; }
{ read -r FIRST; read -r COUNT; read -r EVID; read -r TEST; read -r BASE; read -r SRC; } <<<"$FIELDS"
[ -n "$SRC" ] || SRC=unknown

# 2) the budget — a refusal is RECORDED with its reason, never silently throttled
if [ -z "${PII_INCIDENT_SKIP_BUDGET:-}" ]; then
  REASON=""
  if grep -q "^$DAY " "$DIR/dispatches.log" 2>/dev/null; then REASON="already dispatched on $DAY (one per fingerprint per local day)"; fi
  SPENT=$(awk '{s+=$3} END{printf "%.4f", s+0}' "$DIR/dispatches.log" 2>/dev/null || echo 0)
  if [ -z "$REASON" ] && awk -v s="$SPENT" -v c="$CEILING" 'BEGIN{exit !(s>=c)}'; then REASON="cost ceiling reached: spent $SPENT >= $CEILING USD"; fi
  if [ -n "$REASON" ]; then
    echo "$DAY $(date +%T) refused $REASON" >>"$DIR/refusals.log"
    say "OUTCOME=refused FP=$FP REASON=$REASON"; exit 4
  fi
fi

# 3) the brief — from the incident's own evidence; never a spec
BRIEF="$BRIEFS/$FP.md"
{ echo "# Incident $FP — find the cause and fix it on this branch"
  echo; echo "- fingerprint: $FP"; echo "- source: $SRC"; echo "- first_seen: $FIRST"; echo "- count: $COUNT"; echo "- evidence: $EVID"
  echo "- proving test: ${TEST:-none recorded}"
  echo; echo "Rules: the proving test is RED on the base revision and must be GREEN on your change; the loop runs it itself."
  echo "Do not weaken, skip, delete or rename it. Do not touch specs/. Do not commit. If the premise looks wrong, say so and stop."
} >"$BRIEF"

BASE="${BASE:-$(git rev-parse master)}"
BASE=$(git rev-parse --verify -q "$BASE^{commit}") || { say "OUTCOME=error FP=$FP REASON=base-revision-unknown"; exit 2; }
MASTER_BEFORE=$(git rev-parse master)
BR="incident/$FP"; WT="$DIR/wt"; BWT="$DIR/base"
echo "$DAY $(date +%T) 0" >>"$DIR/dispatches.log"

park(){  # park <reason> — keep the evidence, merge nothing, offer no card
  local why="$1" mb_after; mb_after=$(git rev-parse master)
  git branch -M "$BR" "parked/$FP" 2>/dev/null || true
  say "legs: base_rc=${BASE_RC:--} branch_rc=${BRANCH_RC:--} gate=${GATE:--} master_before=${MASTER_BEFORE:0:12} master_after=${mb_after:0:12}"
  printf 'parked\n%s\n' "$why" >"$DIR/state"
  say "OUTCOME=parked FP=$FP REASON=$why"; exit 1
}

# worktree on incident/<fp> from the recorded base; master is never checked out by this path
if [ -d "$WT/.git" ] || [ -f "$WT/.git" ]; then
  git -C "$WT" checkout -q -B "$BR" 2>/dev/null
else
  git worktree add -q -b "$BR" "$WT" "$BASE" 2>"$DIR/worktree.err" \
    || git worktree add -q -B "$BR" "$WT" "$BASE" 2>>"$DIR/worktree.err" \
    || { say "OUTCOME=error FP=$FP REASON=worktree-failed"; exit 2; }
fi
[ -n "$TEST" ] || { BASE_RC=""; park "no proving test recorded in the incident: nothing can be proven"; }

# 4) the worker
if [ -n "${PII_INCIDENT_WORKER:-}" ]; then WORKER="$PII_INCIDENT_WORKER"; else
  WORKER='claude -p "$(cat "$PII_INCIDENT_BRIEF")" --permission-mode bypassPermissions --model sonnet --output-format json --disallowedTools "Edit(specs/**) Write(specs/**)" >"$PII_INCIDENT_COST_FILE.json" </dev/null; python3 -c "import json,sys; print(json.load(open(sys.argv[1])).get(\"total_cost_usd\",0))" "$PII_INCIDENT_COST_FILE.json" >"$PII_INCIDENT_COST_FILE"'
fi
COSTF="$DIR/cost.$DAY"; : >"$COSTF"
( cd "$WT" && PII_INCIDENT_BRIEF="$BRIEF" PII_INCIDENT_FP="$FP" PII_INCIDENT_COST_FILE="$COSTF" bash -c "$WORKER" ) >"$DIR/worker.log" 2>&1 </dev/null
WRC=$?
COST=$(head -1 "$COSTF" 2>/dev/null); COST=${COST:-0}
case "$COST" in ''|*[!0-9.]*) COST=0;; esac
# record the cost against the line written at dispatch (same day, same fingerprint)
sed -i "s/^\($DAY [0-9:]* \)0\$/\1$COST/" "$DIR/dispatches.log"
[ "$WRC" -eq 0 ] || { BASE_RC=""; park "worker exited rc=$WRC (see $DIR/worker.log)"; }
if git -C "$WT" status --porcelain | awk '{print $NF}' | grep -q '^specs/'; then BASE_RC=""; park "the worker touched specs/ (planner-only)"; fi
if [ -n "$(git -C "$WT" status --porcelain)" ]; then
  git -C "$WT" add -A && git -C "$WT" "${ID[@]}" commit -q -m "incident $FP: fix (proof run by the loop)" || { BASE_RC=""; park "could not commit the worker's change"; }
fi
if [ "$(git -C "$WT" rev-parse HEAD)" = "$BASE" ]; then BASE_RC=""; park "the worker changed nothing"; fi

# 5) the proof: base RED, branch GREEN, gate GREEN — each leg measured here, an unmeasurable leg parks
if [ -e "$BWT/.git" ]; then git -C "$BWT" checkout -q --detach "$BASE"; else git worktree add -q --detach "$BWT" "$BASE" 2>>"$DIR/worktree.err"; fi
( cd "$BWT" && timeout 1780 bash -c "$TEST" ) >"$DIR/base.log" 2>&1; BASE_RC=$?
( cd "$WT" && timeout 1780 bash -c "$TEST" ) >"$DIR/branch.log" 2>&1; BRANCH_RC=$?
GATE=-
unmeasured=""; case "$BASE_RC" in 124|126|127) unmeasured="base";; esac; case "$BRANCH_RC" in 124|126|127) unmeasured="$unmeasured branch";; esac
[ -z "$unmeasured" ] || park "a leg could not be measured ($unmeasured; base_rc=$BASE_RC branch_rc=$BRANCH_RC)"
[ "$BASE_RC" -ne 0 ] || park "the proving test is NOT red on the base revision: the incident's premise is wrong (finding, base_rc=0)"
[ "$BRANCH_RC" -eq 0 ] || park "the proving test is not green on the branch (branch_rc=$BRANCH_RC)"
if ( cd "$WT" && timeout 1780 bash -c "$GATE_CMD" ) >"$DIR/gate.log" 2>&1; then GATE=PASS; else GATE=FAIL; fi
[ "$GATE" = PASS ] || park "the gate is not green (gate=FAIL, see $DIR/gate.log)"

# 6) proven. Merge = fast-forward only; the checkout that holds master (if any) is the one that moves.
TIP=$(git rev-parse "$BR")
if [ "${PII_INCIDENT_MERGE:-1}" = "1" ]; then
  MWT=$(git worktree list --porcelain | awk '/^worktree /{w=$2} /^branch refs\/heads\/master$/{print w}')
  if [ -n "$MWT" ]; then git -C "$MWT" "${ID[@]}" merge -q --ff-only "$BR" >"$DIR/merge.log" 2>&1; MRC=$?
  else git merge-base --is-ancestor master "$BR" && git update-ref refs/heads/master "$TIP" "$MASTER_BEFORE" >"$DIR/merge.log" 2>&1; MRC=$?; fi
  [ "$MRC" -eq 0 ] || park "fast-forward refused (master moved or its tree is dirty; see $DIR/merge.log)"
  VERDICT=merged
else VERDICT=proven; fi
MASTER_AFTER=$(git rev-parse master)
say "legs: base_rc=$BASE_RC branch_rc=$BRANCH_RC gate=$GATE master_before=${MASTER_BEFORE:0:12} master_after=${MASTER_AFTER:0:12}"
printf '%s\n%s\n' "$VERDICT" "$TIP" >"$DIR/state"
say "OUTCOME=$VERDICT FP=$FP BRANCH=$BR TIP=${TIP:0:12}"
exit 0
