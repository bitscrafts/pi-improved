#!/usr/bin/env bash

# The tree hold (2026-11-01): a cycle writes the tree -- worker edits, commits, gate exports -- and an interactive
# turn can be holding it with no lock of its own. A cron-side cycle yields with one line and exit 0, the same benign
# refusal drain.sh makes for a held lock, so a held tree never looks like a fault.
if [ "${PII_HOLD_FROM_CRON:-0}" = 1 ] || [ "${PII_CRON:-}" = 1 ]; then
  bash scripts/autonomy/hold.sh check >/dev/null 2>&1
  if [ $? -eq 3 ]; then
    bash scripts/autonomy/hold.sh check | sed 's/^hold: /cycle: /'
    echo "OUTCOME=tree-held"; exit 0
  fi
fi
# THE LOCK SPANS THE WHOLE CYCLE (2026-10-15). The launcher exits immediately, so a lock taken by the launcher is
# released the moment the cycle starts: that is how four cycles ran at once and their builds queued on cargo.
if [ -z "${PII_CYCLE_LOCKED:-}" ]; then
  export PII_CYCLE_LOCKED=1
  # not `exec flock ... || ...`: exec replaces this shell, so the || never ran and a held lock exited 1, silently
  # -o (--close) so a daemonized grandchild cannot keep the lock (measured 2026-11-04): sccache,
  # started by the cycle's own cargo and re-parented to init, inherited this fd and held the lock
  # after the cycle was gone, so every later drain read "a cycle is in flight" and dispatched nothing.
  flock -n -o -E 75 "${PII_LOCK:-/projects/pi-improved/_tmp/autonomy/lock}" bash "$0" "$@"; rc=$?
  [ "$rc" = 75 ] && { echo "OUTCOME=locked ITEM=-"; exit 5; }
  exit "$rc"
fi
# The age control.sh reports ("IN FLIGHT for N min") is the lock FILE's mtime, and flock never updates it: on
# 2026-10-19 it read 6073 min while the lock was free, were it believed. Touch it at the moment this process
# starts holding the lock, so the number means the age of THIS hold and nothing else.
touch "${PII_LOCK:-/projects/pi-improved/_tmp/autonomy/lock}" 2>/dev/null || true
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
. "$(dirname "$(readlink -f "$0")")/env.sh"   # the loop needs PATH set inside the script
. "$(dirname "$(readlink -f "$0")")/gate-env.sh"   # spec 262: which step list a queue item's commit is gated with
set -uo pipefail
ROOT=/projects/pi-improved; cd "$ROOT" || exit 1
STATEDIR=$ROOT/_tmp/autonomy; LOGD=$STATEDIR/logs
mkdir -p "$LOGD" _tmp/claude-batchC
CID=$(date -u +%Y%m%dT%H%M%SZ)
LOG=$LOGD/cycle-$CID.log
say(){ printf '[%s] %s\n' "$(date -u +%H:%M:%SZ)" "$*"; }
say "=== cycle $CID"
. "$ROOT/scripts/autonomy/report.sh"   # spec 173: lifecycle lines go to pii as events (fail-open)
# The launcher above already holds the lock for this process. A second flock on a NEW descriptor of the same file
# conflicts with it (flock is per open file description), so re-taking it here refused every cycle (02:01-04:01).
if [ "${PII_CYCLE_LOCKED:-}" != 1 ]; then
  exec 9>"${PII_LOCK:-$STATEDIR/lock}"
  if ! flock -n 9; then say "another cycle holds the lock — nothing done"; echo "OUTCOME=locked"; exit 5; fi
fi

# 0b) the operator's switch (Discord: `scripts/autonomy/control.sh pause`) — honour it before spending
if grep -q PAUSED "${PII_CONTROL:-$STATEDIR/control}" 2>/dev/null; then
  say "PAUSED by the operator — nothing spent, nothing advanced"; echo "OUTCOME=paused"; exit 0
fi

# 1) the window
PROBE=$(bash scripts/autonomy/probe.sh 2>&1); PRC=$?
echo "$PROBE" >>"$LOG"
case $PRC in
  0) say "probe: $(printf '%s' "$PROBE" | head -1)";;
  3) say "QUOTA CLOSED — nothing spent, the item was NOT advanced"; say "$PROBE"; echo "OUTCOME=window-closed"; exit 3;;
  *) say "PROBE UNDECIDED rc=$PRC — stopping rather than guessing about the window"; say "$PROBE"; echo "OUTCOME=probe-error"; exit 4;;
esac

# 2) the plan
# PII_FORCE_ITEM names the row this cycle must work, without consulting the pick: the drain sets it when
# the tree already holds that row's unfinished work, and only that row may advance from that tree.
if [ -n "${PII_FORCE_ITEM:-}" ]; then
  # the drain named the row that owns the dirty tree: only that row may advance from this tree, and the
  # pick is not consulted (it would skip a needs-review row and leave the work sitting again).
  # `state.py pick --row` — the pick's own checks and its 9-column output for the NAMED row. `state.py get`
  # returns the row's status JSON, and gluing it behind the id gave a "spec" of `{` and an empty title, so a
  # forced item always ended in needs-spec (measured 2026-11-05 on the first forced dispatch).
  PICK=$(python3 scripts/autonomy/state.py pick --row "$PII_FORCE_ITEM")
  if [ -z "$PICK" ] || [ "$PICK" = "NONE" ]; then
    say "the forced item '$PII_FORCE_ITEM' is not dispatchable (not in plan.tsv, or its paths/criteria are refused) — nothing spent"
    echo "OUTCOME=plan-error"; exit 4
  fi
  say "item forced by the dispatcher: $PII_FORCE_ITEM (the tree holds its work)"
else
  PICK=$(python3 scripts/autonomy/state.py pick)
fi
# 2a-1) a plan that cannot be READ is a loud failure, never an empty queue: state.py refuses a duplicate row id,
# and "nothing pending" and "the queue is broken" are different outcomes — only one of them is safe to sleep on.
if [ -z "$PICK" ]; then
  say "the plan could not be read — state.py refused it (see its message above). NOTHING dispatched; this is NOT an empty queue"
  echo "OUTCOME=plan-error"; exit 4
fi
if [ "$PICK" = "NONE" ]; then
  # 2a) the second plan source (spec 151), after plan.tsv: each open incident (no state yet under incidents/<fp>/)
  # goes to incident-fix.sh, which works on a branch and parks or merges only on its own proof. DRY=1 only lists.
  INCF=${PII_INCIDENTS:-$STATEDIR/incidents.jsonl}; [ -f "$INCF" ] || INCF=$STATEDIR/logs/incidents.jsonl
  if [ -f "$INCF" ]; then
    mapfile -t OPEN < <(python3 -c 'import json,sys
seen=[]
for l in open(sys.argv[1]):
    try: f=json.loads(l).get("fingerprint")
    except Exception: continue
    if f and f not in seen: seen.append(f)
print("\n".join(seen))' "$INCF")
    for FP in "${OPEN[@]}"; do
      [ -n "$FP" ] && [ ! -f "$STATEDIR/incidents/$FP/state" ] || continue
      if [ -n "${DRY:-}" ]; then say "DRY=1: would work incident $FP"; echo "OUTCOME=dry-incident FP=$FP"; continue; fi
      say "incident $FP: working it on its own branch"
      bash scripts/autonomy/incident-fix.sh "$FP" 2>&1 | tee -a "$LOG"
    done
  fi
  # THE QUEUE-END BRIEF (2026-10-19, user request: "I do also need a brief on queue end! saying what the session did!").
  # Emitted HERE, at the moment the queue empties, because a brief produced on request is a brief nobody reads when it
  # matters. It states the counts, the rows that are NOT done, and the commits — and it writes a dated artefact, so the
  # session ends with a file rather than a paragraph in a terminal.
  # One file per HOUR, overwritten each pass. Measured 2026-10-19: the drain loop asked 40 times and left 33 near-identical
  # briefs in _docs, which is file spam dressed as reporting. A brief summarises the session, not the iteration.
  BRIEF=$(bash scripts/autonomy/brief-path.sh)
  bash scripts/autonomy/queue-brief.sh > "$BRIEF" 2>/dev/null || echo "queue brief: FAILED to generate" >> "$BRIEF"
  say "queue empty — nothing pending"
  say "queue brief: $BRIEF"
  say "$(grep -c '^  A' "$BRIEF" 2>/dev/null || echo 0) row(s) NOT done — listed in the brief"
  echo "OUTCOME=queue-empty"; exit 0
fi
# A tab is IFS whitespace, so read merges consecutive tabs and an empty column shifts every later field (measured
# 2026-10-15: A25's empty gate put its session id into ATTEMPTS, the done-arithmetic failed, and the item was
# re-verified 23 times without ever closing). Split on a non-whitespace separator so an empty field stays empty.
IFS=$'\x1f' read -r ITEM SPEC BRIEF TITLE VERIFY PATHS GATE ATTEMPTS RSID <<<"${PICK//$'\t'/$'\x1f'}"
say "item=$ITEM spec=$SPEC attempts=$ATTEMPTS title=$TITLE"
report loop.cycle.start "item=$ITEM spec=$SPEC attempts=$ATTEMPTS"
if [ ! -f "specs/$SPEC" ]; then
  say "NO SPEC at specs/$SPEC — the agent must AUTHOR it before this cycle can spend anything"
  echo "OUTCOME=needs-spec ITEM=$ITEM TITLE=$TITLE"; exit 6
fi
# A spec NUMBER must resolve to exactly ONE file. The dispatcher resolves `ls specs/<N>-*.md | head -1` and
# session.sh recover keys the resume handle by that same number, so two specs sharing a number send the item to
# the WRONG file and resume ANOTHER item's finished session. Measured 2026-10-18: item A52 (183-the-worker-node)
# was dispatched as 183-the-lagged-chain — already committed — resumed its 10-turn session, spent $0.37, changed
# nothing, and then failed verify on its own criterion. A duplicate number is a plan defect: refuse before spending.
NDUP=$(ls specs/"$(printf '%s' "$SPEC" | sed -E 's/^([0-9]+).*/\1/')"-*.md 2>/dev/null | wc -l)
if [ "$NDUP" -ne 1 ]; then
  say "spec number in $SPEC matches $NDUP files under specs/ — the dispatcher would pick the first and recover another item's session; refusing"
  echo "OUTCOME=plan-error ITEM=$ITEM SPEC=$SPEC DUPLICATE-NUMBER"; exit 4
fi
if [ -n "$BRIEF" ] && [ "$BRIEF" != "-" ] && [ ! -f "$BRIEF" ]; then
  say "NO BRIEF at $BRIEF"; echo "OUTCOME=needs-brief ITEM=$ITEM"; exit 6
fi
[ -n "${DRY:-}" ] && { say "DRY=1: would dispatch spec=$SPEC brief=$BRIEF resume=${RSID:-none}"; echo "OUTCOME=dry"; exit 0; }

# 2b) re-read the state at the LAST moment: between the pick and this line another cycle (or the orchestrator)
# may have closed the item, and re-dispatching finished work buys the same spec twice.
ST_NOW=$(python3 scripts/autonomy/state.py get "$ITEM" | python3 -c 'import json,sys; d=json.load(sys.stdin); print(d.get("status",""), d.get("last",""))' 2>/dev/null)
case "$ST_NOW" in
  done*|skipped*|parked*)
    say "item $ITEM is already ${ST_NOW%% *} — not dispatching (nothing else to do)"; echo "OUTCOME=item-already-closed ITEM=$ITEM"; exit 0 ;;
esac
# a phase marker from a cycle that died mid-flight: if the work is already worker-ok/verified/gated and the tree
# still holds it, resume at verify instead of buying the same spec again
SKIP_DISPATCH=""
case "$ST_NOW" in
  *verified*|*gated*|*worker-ok*) SKIP_DISPATCH=1; say "phase marker says the work is already ${ST_NOW#* }: resuming at verify, no re-dispatch" ;;
esac

# 3) the worker — resume-first, the session id comes from the state, not from memory
DIRTY=$(git status --porcelain | wc -l)
say "tree: $DIRTY changed paths before dispatch"
if [ "$DIRTY" -ne 0 ] && [ -z "$RSID" ]; then
  # A leftover from the last item must not stop the loop (measured 2026-10-13: spec 139's guide update was not
  # in its plan row's paths, so the tree stayed dirty and every cycle refused - the loop idled for hours).
  # Keep it saved (leftover.sh, spec 311), clean only the row the loop owns, and carry on: nothing is lost.
  # Before treating a dirty tree as a leftover, look for a session to RESUME: a worker cut off mid-flight leaves
  # its session id in its own log even when the dispatcher never got to write it (measured 2026-10-13).
  RECOVER=$(bash scripts/autonomy/session.sh recover "$(printf '%s' "$SPEC" | sed -E 's/^([0-9]+).*/\1/')" 2>/dev/null || true)
  if [ -n "$RECOVER" ]; then
    python3 scripts/autonomy/state.py set "$ITEM" status=needs-review "session=$RECOVER" "last=resumed-from-worker-log"
    say "dirty tree WITH a session recovered from the worker's log ($RECOVER): resuming, not restarting"
    RSID="$RECOVER"; ARGS=(--resume "$RECOVER")
  else
  # spec 311: SAVE first (a clean without a save is the only unrecoverable path), then ATTRIBUTE, then CLEAN.
  # The row the loop itself left unfinished is stashed, so this worker's gate measures only its own change
  # (measured 2026-10-29: A174 inherited A173's leftover and its gate failed on A173's test). A tree no row owns
  # is a person's: nothing is cleaned and no worker is bought beside it (GATE BYPASS CLOSED 2026-10-20: never committed).
  LEFT=$STATEDIR/leftovers
  if ! bash scripts/autonomy/leftover.sh save "$LEFT" "$CID" >/dev/null; then
    say "could not save the leftover under $LEFT: nothing cleaned, nothing dispatched"; echo "OUTCOME=dirty-tree ITEM=$ITEM"; exit 7
  fi
  OWN=$(bash scripts/autonomy/leftover.sh owner); ORC=$?
  if [ "$ORC" = 0 ]; then
    CLEANED=$(bash scripts/autonomy/leftover.sh clean "$CID"); CRC=$?
    say "leftover of ${OWN#OWNER } saved under $LEFT, tree cleaned: $CLEANED"
    if [ "$CRC" != 0 ]; then echo "OUTCOME=dirty-tree ITEM=$ITEM"; exit 7; fi
  else
    say "DIRTY TREE that no queue row owns ($OWN): saved under $LEFT ($(ls "$LEFT/$CID"-* 2>/dev/null | xargs -n1 basename | tr '\n' ' ')), nothing cleaned, nothing dispatched"
    bash scripts/autonomy/control.sh ask "a dirty tree no queue row owns was saved under $LEFT (cycle $CID) and the cycle stopped: nothing cleaned, nothing committed." >/dev/null 2>&1 || true
    echo "OUTCOME=dirty-tree ITEM=$ITEM"; exit 7
  fi
  fi
fi
ARGS=(); [ -n "$RSID" ] && ARGS=(--resume "$RSID")
BRIEFARG=(); [ -n "$BRIEF" ] && [ "$BRIEF" != "-" ] && BRIEFARG=("$BRIEF")
# A resume after a failed attempt carries a corrective brief: the worker cannot see the gate's verdict, and
# its own criteria may well have passed (that is exactly how spec 133's first pass went — criteria 4/4, gate
# RED on a tree the export had not included). Naming the failure is the difference between a fix and a retry.
if [ -n "$RSID" ]; then
  PRV=$(python3 scripts/autonomy/state.py get "$ITEM" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("last",""))' 2>/dev/null)
  case "$PRV" in
    *gate-failed*|*verify-failed*|*worker-rc*)
      FB="$STATEDIR/briefs/corrective-$ITEM-$CID.md"; mkdir -p "$STATEDIR/briefs"
      # The log of the cycle that FAILED — not this one, and not the accumulator. Measured 2026-11-04: this
      # line was `ls -t "$LOGD"/cycle-*.log | head -1`, and the accumulator cycle-last-detached.log is being
      # appended to while the brief is built, so it was usually the newest — while its OUTCOME lines span
      # months and it carries no `crit=` verify line at all. The brief for row 370 therefore quoted rows
      # A1..A7 and window-closed from a previous month. The failure detail lives in the per-cycle log of the
      # cycle that just ended, which is the newest one that is neither this cycle's nor the accumulator.
      LASTCYCLE=$(ls -t "$LOGD"/cycle-*.log 2>/dev/null | grep -v '/cycle-last-detached\.log$' \
                  | grep -v "/cycle-$CID\.log$" | head -1)
      { echo "# Corrective brief — $ITEM ($SPEC). The previous attempt ended: $PRV"; echo
        echo "Your earlier pass on this spec did not survive the orchestrator's check. Evidence:"
        echo '```'
        # The verify line is shaped `<n> verify - … crit=<p>/<t> fail=<command> rc=<-`. None of the gate's
        # patterns match it, so this block came out EMPTY and a worker retried blind three times (measured
        # 2026-11-01, row 330: "the brief's evidence block is empty again"). brief-evidence.sh matches the
        # shapes the cycle really writes and falls back to the log's tail, so it can never print nothing.
        bash scripts/autonomy/brief-evidence.sh "$LASTCYCLE"
        echo '```'
        echo "Fix the CAUSE, not the symptom: do not weaken, skip, ignore or delete an assertion, do not raise"
        echo "a timeout, do not add a sleep, and do not touch anything outside this spec's scope."
        echo "Re-read the spec's TDD Contract and Exit Criteria and leave the tree with those criteria passing."
      } >"$FB"
      BRIEFARG=("$FB"); say "resume carries a corrective brief: $FB"
      ;;
  esac
fi
# the dispatcher takes a spec NUMBER, not a filename (it resolves specs/<N>-*.md itself)
N=$(printf '%s' "$SPEC" | sed -E 's/^([0-9]+).*/\1/')
[ -n "$N" ] || { say "cannot read a spec number out of '$SPEC'"; echo "OUTCOME=bad-plan ITEM=$ITEM"; exit 6; }
# spec 304: WLOG is resolved for BOTH paths, before the dispatch branch: a resume at verify (SKIP_DISPATCH) buys nothing and reaches the
# commit with this handle. The dispatch path resolves it again below, once its own run has written its log.
# spec 354: a PARK MUST NOT WEDGE THE QUEUE. Parking used to leave the row's working-tree changes in place, and the
# dispatcher refuses to dispatch on a dirty tree, so one parked row stopped every later row for a whole night
# (measured 2026-11-03: spec 352 parked with nine files and 353 never ran). The changes are copied aside and the
# tree is returned to HEAD — never deleted, and never with `git checkout --`, which is banned in this workspace.
park_and_quarantine() {
  local note="$1" aside
  aside=$(bash scripts/autonomy/quarantine-wip.sh "$ITEM" parked 2>/dev/null | tail -1)
  case "$aside" in
    CLEAN)  say "park: tree already clean" ;;
    /*)     say "park: WIP preserved at $aside; the tree is clean, so the queue can move" ; note="$note (WIP: $aside)" ;;
    *)      say "park: WARNING the tree could not be quarantined ($aside) — the dispatcher will refuse until it is clean" ;;
  esac
  python3 scripts/autonomy/state.py set "$ITEM" "last=$note"
}

WLOG=$(ls -t _tmp/claude-batchC/"$N"-*.log 2>/dev/null | head -1)
if [ -n "$SKIP_DISPATCH" ]; then
  NEWSID="$RSID"
else
  if [ "${ATTEMPTS:-0}" -ge 3 ]; then
    say "attempts=$ATTEMPTS reached the cap of 3 — PARKING for a person, not dispatching again"
    python3 scripts/autonomy/state.py set "$ITEM" status=parked attempts="$ATTEMPTS" "last=parked after $ATTEMPTS attempts: the same mechanical failure repeated, so a fourth dispatch would spend without new information"
    park_and_quarantine "parked after $ATTEMPTS attempts"
    report loop.park "item=$ITEM attempts=$ATTEMPTS"
    echo "OUTCOME=parked-attempt-cap ITEM=$ITEM"; exit 11
  fi
  say "dispatching spec number $N"
  # REVIEW ROWS (user directive 2026-10-19: "an opus analysis must also be put in the queue, positioned before the items
  # that matter"). A row whose spec is specs/review-* is dispatched to the opus reviewer in plan mode, never to a coder.
  # A DEPLOY row is executed by the dispatcher, not by a coding worker: its spec carries `DEPLOY:` on its own line.
  # SPEC is the FILENAME, so the file lives at specs/$SPEC — measured again 2026-10-20: grepping "$SPEC" found nothing, the
  # DEPLOY branch never fired, and the row went down the worker path where its verify command ran a full install.
  if grep -qE '^DEPLOY:' "specs/$SPEC" 2>/dev/null; then
    say "deploy row: running scripts/autonomy/deploy-row.sh for $SPEC"
    if bash scripts/autonomy/deploy-row.sh >"_tmp/autonomy/logs/deploy-$ITEM-$(date -u +%Y%m%dT%H%M%SZ).log" 2>&1; then
      echo "OUTCOME=deploy-run ITEM=$ITEM"
    else
      echo "OUTCOME=deploy-failed ITEM=$ITEM"
    fi
    exit 0
  fi

  # SPEC is the FILENAME, not a path (measured 2026-10-19: the specs/216-* pattern never matched and a coding worker was
  # dispatched onto a review spec). Match both forms so this cannot recur.
  case "$SPEC" in
    *review-*|*216-*|specs/review-*|specs/216-*)
      say "review row: dispatching the reviewer for $SPEC"
      ( bash scripts/autonomy/review-row.sh "$N" ) >>"$LOG" 2>&1
      RC=$?
      say "reviewer rc=$RC"
      ;;
    *)
  # NOTE (2026-10-19): the quota test below matches the CLI's OWN error shapes, not the words "quota"/"rate limit" anywhere
# in a log — a brief that QUOTES the user's directive about the quota produced two false requeues tonight (A80, A82).
  # A row whose spec declares DEPLOY: must NEVER reach this point: if it did, the deploy branch above failed to match and
  # the row would be dispatched as code and its verify command would run an install (measured 2026-10-20). Fail loudly.
  if grep -qE '^DEPLOY:' "specs/$SPEC" 2>/dev/null; then
    echo "OUTCOME=plan-error ITEM=$ITEM SPEC=$SPEC DEPLOY-ROW-REACHED-WORKER-PATH"
    exit 1
  fi

  # WORKER SELECTION (user directive 2026-10-19: "dont implement it as the anthropic quota will exaust").
  # PII_WORKER=pi (default) dispatches to ollama-cloud through scripts/run-batchC-pi.sh — the Claude account is untouched.
  # PII_WORKER=claude is the opt-in escalation, and only when the window is known open.
  # PII_WORKER=pii (the default since 2026-10-24, user directive "all our specs should be implemented by pii") runs the item as a
  # coder-node graph through the pii daemon inside the devbox container (scripts/run-batchC-pii.sh; PII_AGENT picks claude or pi).
  if [ "${PII_WORKER:-pii}" = "pii" ]; then
    ( ALLOW_DIRTY=1 bash scripts/run-batchC-pii.sh "$N" "${ARGS[@]}" "${BRIEFARG[@]}" ) >>"$LOG" 2>&1
  elif [ "${PII_WORKER:-pii}" = "claude" ]; then
    ( ALLOW_DIRTY=1 bash scripts/run-batchC-claude.sh "$N" "${ARGS[@]}" "${BRIEFARG[@]}" ) >>"$LOG" 2>&1
  else
    ( ALLOW_DIRTY=1 bash scripts/run-batchC-pi.sh "$N" "${BRIEFARG[@]}" ) >>"$LOG" 2>&1
  fi
      ;;
  esac
  WRC=$?
  NEWSID=$(awk '{print $1}' _tmp/claude-batchC/session-"$N"-*.txt 2>/dev/null | tail -1)
  say "worker rc=$WRC session=${NEWSID:-none}"
  # The worker may have been refused by the account's WINDOW rather than stopped by its work. The 1-turn probe can
  # say OPEN while the worker is refused (measured 2026-10-13), so read the worker's OWN log for the CLI's words and
  # honour the reset it states — never blind-retry, never shrink scope (the project's oldest operational rule).
  WLOG=$(ls -t _tmp/claude-batchC/"$N"-*.log 2>/dev/null | head -1)
  # lane-record:begin
  # spec 304: the lane this run used is the LAST `LANE-GRAPH:` line of its own log; it is recorded on the row only when the run changed the
  # tree (the first coder.exited, the host's, as the FACTS line reads it). A run that changed nothing leaves the previous record standing,
  # so a commit is never attributed to a run that produced no tree of its own.
  RUN_LANE=$(grep -a '^LANE-GRAPH: ' "${WLOG:-}" 2>/dev/null | tail -1 | sed -n 's/^LANE-GRAPH: \(spec\|coder\)$/\1/p')
  RUN_CHANGED=$(grep -a -m1 '^== coder.exited: ' "${WLOG:-}" 2>/dev/null | grep -o '"tree_changed":"[a-z]*"' | head -1 | cut -d'"' -f4)
  if [ "$RUN_CHANGED" = yes ] && [ -n "$RUN_LANE" ]; then python3 scripts/autonomy/state.py set "$ITEM" "lane=$RUN_LANE"; fi
  # lane-record:end
  report loop.dispatch "item=$ITEM session=${NEWSID:-none} rc=$WRC lane=${RUN_LANE:-none}"
  WUNTIL=$(bash scripts/autonomy/window.sh parse "$WLOG" 2>/dev/null || true)
  if [ -n "$WUNTIL" ]; then
    echo "$WUNTIL" > "$STATEDIR/window-closed-until"
    say "the account window is closed (the worker's own log said so) — not retrying before $(date -u -d "@$WUNTIL" '+%H:%M UTC')"
    # RECORD the session before leaving: a worker cut off by the window must be RESUMED, not restarted. Without this
    # the next cycle saw a dirty tree with no session, kept the WIP as a leftover and dispatched the spec again from
    # scratch (measured 2026-10-13, spec 153).
    [ -n "${NEWSID:-}" ] && python3 scripts/autonomy/state.py set "$ITEM" status=needs-review "session=$NEWSID" last=window-closed
    echo "OUTCOME=window-closed ITEM=$ITEM SESSION=${NEWSID:-}"
    exit 3
  fi
  # AUTH IS A LANE FAULT, NOT A ROW FAULT (spec 254): a dead login refuses every row, so counting it as an attempt parks
  # three rows that were never tried. The row keeps its attempts, the control file stops the queue, and a person acts.
  if [ "$WRC" -ne 0 ] && [ -n "${WLOG:-}" ] && [ "$(bash scripts/autonomy/classify-failure.sh <"$WLOG" 2>/dev/null)" = auth ]; then
    python3 scripts/autonomy/state.py set "$ITEM" status=needs-review "session=${NEWSID:-}" last=lane-auth-failed
    bash scripts/autonomy/control.sh pause "lane-auth-failed $(date -u '+%FT%TZ')" >/dev/null 2>&1 || true
    say "AUTH FAILURE: the worker's login is dead — the row keeps its attempts and the queue is PAUSED for a person"
    echo "OUTCOME=lane-auth-failed ITEM=$ITEM"; exit 16
  fi
  # A DISPATCH THAT NEVER STARTED IS NOT AN ATTEMPT. The lane exits 8 when it refuses a dirty tree,
  # and 9 when it refuses before spending; in both cases nothing was bought and nothing was tried, so the
  # row keeps its status and its attempt count. Measured 2026-11-05: recording those as `worker-rc=8`
  # gave eight untouched rows a worker failure, and the queue looked like it had failed eight times.
  case "$WRC" in
    8|9)
      python3 scripts/autonomy/state.py set "$ITEM" status=pending "session=${NEWSID:-}" last=lane-not-started-rc=$WRC
      say "the lane did not start (rc=$WRC) — the row keeps its place and its attempts; nothing was spent"
      echo "OUTCOME=lane-not-started ITEM=$ITEM RC=$WRC"; exit 8 ;;
  esac
  python3 scripts/autonomy/state.py set "$ITEM" status=needs-review "session=${NEWSID:-}" last=worker-rc=$WRC attempts=$((ATTEMPTS+1))
  [ $WRC -ne 0 ] && { say "worker did not finish cleanly — leaving the WIP for the next cycle to resume"; echo "OUTCOME=worker-rc-$WRC ITEM=$ITEM SESSION=${NEWSID:-}"; exit 8; }
  # QUOTA IS NOT A FAILURE (user rule 2026-10-19: "eventually it will exaust, let it run to the end then put it again on
  # the queue"). A spend/rate limit must not park the row for a person: capture the reset the CLI itself printed, put the
  # row BACK on the queue with its attempts and session intact, and let the next pass resume it.
  if [ -n "${WLOG:-}" ] && grep -qiE "you've hit your (session|weekly|monthly|usage) limit|\"rate_limit_error\"|\"type\":\"rate_limit\"|resets [0-9]{1,2}:[0-9]{2}" "$WLOG" 2>/dev/null; then
    RESET=$(grep -oiE '(resets? (at )?[^.,;]{0,40})' "$WLOG" 2>/dev/null | head -1)
    python3 scripts/autonomy/state.py set "$ITEM" status=pending attempts="$ATTEMPTS" \
      "last=requeued: provider limit hit (${RESET:-reset time not printed}) — resume the same session, do not restart" >/dev/null 2>&1
    say "QUOTA: $ITEM back on the queue — ${RESET:-reset unknown}"
    echo "OUTCOME=requeued-quota ITEM=$ITEM SESSION=${RECOVER:-}"; exit 0
  fi
  python3 scripts/autonomy/state.py set "$ITEM" last=worker-ok "session=${NEWSID:-}"
fi

# 4) the item's own criteria — run from the plan, on the tree the worker left
VFAIL=""
if [ -n "$VERIFY" ] && [ "$VERIFY" != "-" ]; then
  IFS=';' read -ra CMDS <<<"$VERIFY"
  for c in "${CMDS[@]}"; do
    say "verify: $c"
    # The item's command runs in a LOGIN shell on purpose (the operator's environment), but /etc/profile
    # RESETS PATH for root to `/usr/local/sbin:/usr/local/bin:...` — no /usr/local/cargo/bin — so the loop's
    # own PATH contract (env.sh) is gone by the time the command runs. Measured 2026-10-16: `bash -lc 'command
    # -v cargo'` answers nothing, while the same criterion returns rc=0 with cargo on PATH; A26's two cargo
    # criteria were therefore recorded verify-failed twice on a tree where all four of its criteria pass.
    # Source env.sh INSIDE the login shell so the item is verified with the tools the cycle itself has.
    if bash -lc ". '$ROOT/scripts/autonomy/env.sh'; $c" >>"$LOG" 2>&1; then say "  rc=0"; else say "  rc!=0"; VFAIL="$c"; fi
  done
fi
if [ -n "$VFAIL" ]; then
  python3 scripts/autonomy/state.py set "$ITEM" status=needs-review attempts=$((ATTEMPTS+1)) "last=verify-failed"
  [ $((ATTEMPTS+1)) -ge 3 ] && { python3 scripts/autonomy/state.py set "$ITEM" status=parked "last=3-strikes"; report loop.park "item=$ITEM 3-strikes"; park_and_quarantine "3-strikes at verify"; }
  report loop.verify "item=$ITEM FAILED: $VFAIL"
  say "VERIFY FAILED: $VFAIL"; echo "OUTCOME=verify-failed ITEM=$ITEM CMD=$VFAIL"; exit 9
fi

report loop.verify "item=$ITEM ok"

# 5) the gate — a green verify is not a green gate
# Only an explicit 0 / - / none skips the gate: rows A22-A24 named "quality-gate.sh" here, failed an `= 1` test, and
# were committed with no fresh-gate under a message that said "gate green" (075fee9). Fail toward gating.
case "${GATE:-1}" in 0|-|none) GATE_ON= ;; *) GATE_ON=1 ;; esac
if [ -n "$GATE_ON" ]; then
  # spec 273: the floor checks every volume the gate writes to (GATE_WRITE_DIRS), not /projects alone
  # ${GATE_WRITE_DIRS:-}: unset in production, so an unbound expansion under `set -u` failed the
  # command and the failure was reported as a disk shortage -- measured 2026-11-04, the gate was
  # skipped on a row with 127 GB free. A check must not turn its own defect into a diagnosis.
  # The cycle owns the dirs its gate writes to: make them first, so the floor judges ROOM and not whether a clean
  # removed a directory (measured 2026-11-05: a finished row's gate skipped on 112 GB free, "MISSING llvm-cov-target").
  bash scripts/autonomy/ensure-write-dirs.sh ${GATE_WRITE_DIRS:-} || say "write dir(s) could not be made; the floor will report them"
  DISK=$(bash scripts/autonomy/disk-floor.sh 20000 /projects ${GATE_WRITE_DIRS:-}); DRC=$?
  if [ "$DRC" -ne 0 ]; then
    LOWL=$(echo "$DISK" | grep -m1 -E '^(LOW|MISSING): ')
    FREE=$(echo "$LOWL" | sed -n 's/^LOW: .* free=\([0-9]*\)M .*/\1/p')
    # A skipped gate must NOT fall through to the commit, and must NOT advance the row: nothing was tried (spec 395).
    if [ "$DRC" -eq 4 ]; then WHYSKIP=wiring; say "WIRING: $LOWL — a write dir is absent; this is not a disk shortage"; else WHYSKIP=disk; fi
    python3 scripts/autonomy/state.py set "$ITEM" status=pending last=gate-skipped:$WHYSKIP
    say "gate SKIPPED ($WHYSKIP): $LOWL — the item is NOT committed without a gate and stays pending"
    echo "OUTCOME=gate-skipped-$WHYSKIP ITEM=$ITEM ADVANCED=no (row not advanced) FREE_MB=$FREE WHY=$(printf %s "$DISK" | tr '\n' ' ')"
    bash scripts/autonomy/control.sh ask "disk is down to ${FREE}M: the gate cannot run, so $ITEM cannot be committed. Reclaim with cargo clean (an operator action here)?" >/dev/null 2>&1 || true
    exit 13
  else
    say "routine gate (spec 171: repo tree, warm cache; --fresh is the release boundary, run before a deploy)"
    # A full export gate MEASURES ~45-48 min on this host (2026-10-16 22:53:33 → 23:41 = 47.5 min for the 14
    # steps): every export is a NEW source path (`fresh-gate-<pid>`), so cargo's fingerprints miss and the step
    # list recompiles the workspace. The old cap was 1780 s — shorter than every run since 2026-10-15 — so each
    # gated item ended `gate-failed` with its tree never judged. 3600 s leaves the measured run room and still
    # bounds a hang: every step also kills itself after QG_STALL_SECS=1800 with no output.
    GATE_TIMEOUT=${GATE_TIMEOUT:-3600}
    # spec 262: the commit is gated with the FULL list, pinned on this command (env.sh exports the quick list for other callers).
    # spec 266: the skip switch is pinned off here too (gate_skip_coverage prints 0, the QG_SKIP_COVERAGE=0 this line carries).
    GLIST=$(gate_steps_file); GCOUNT=$(gate_step_labels "$GLIST" | wc -l | tr -d ' ')
    say "gate steps: $(gate_step_labels "$GLIST" | paste -sd, - | sed 's/,/, /g') ($GCOUNT steps)"
    # 2026-11-06: a PII_BUILD_REVISION left in the launching shell is honoured by pii-server's build.rs BEFORE git, so every
    # binary in the gate was stamped with a stale commit and lane_run_parity (rightly) refused it. The gate stamps from git.
    ( unset PII_BUILD_REVISION; export PATH="/usr/local/cargo/bin:$PATH"; GATE_STEPS=gate-steps.txt QG_SKIP_COVERAGE=$(gate_skip_coverage) CARGO_TARGET_DIR=/projects/pi-improved/_tmp/gate-target FRESH_GATE_MIN_FREE_MB=12000 timeout "$GATE_TIMEOUT" bash quality-gate.sh --routine ) >>"$LOG" 2>&1
    GTRC=$?
    if [ "$GTRC" -eq 0 ]; then
      # spec 266: a green gate that never measured coverage is not a pass for a queue commit
      COVM=$(coverage_measured "$LOG")
      if [ "$COVM" = yes ]; then say "gate coverage: measured"; else say "gate coverage: NOT MEASURED"; fi
      if [ "$COVM" = no ]; then
        python3 scripts/autonomy/state.py set "$ITEM" status=needs-review last=gate-coverage-not-measured
        report loop.gate.verdict "item=$ITEM coverage NOT MEASURED"
        say "gate passed but coverage was NOT measured — the item is NOT committed"; echo "OUTCOME=gate-coverage-not-measured ITEM=$ITEM"; exit 16
      fi
      python3 scripts/autonomy/state.py set "$ITEM" last=gated; say "gate PASS ($GCOUNT steps) (phase marked: gated)"
      report loop.gate.verdict "item=$ITEM pass"
    elif [ "$GTRC" -eq 124 ]; then
      # A KILLED gate is not a verdict. Decided 2026-10-14 (DECISION-fresh-worktrees-and-a-resumable-gate-build.md
      # §2): a timeout gets its own outcome, because "FAILED" must mean the tree is wrong, and a gate that says
      # FAILED while the tree is fine is the same lie as a check that passes while its path is broken. Attempts
      # are NOT incremented and the marker keeps the next cycle at VERIFY, so a slow host costs a retry — never a
      # worker re-dispatch (which would re-bill a paid run) and never a park dressed as a code failure.
      python3 scripts/autonomy/state.py set "$ITEM" status=needs-review "last=worker-ok; gate-timeout after ${GATE_TIMEOUT}s — the gate was KILLED, not failed; retried at verify"
      say "gate TIMED OUT after ${GATE_TIMEOUT}s — NOT MEASURED, not a failure; the item is retried at verify"
      echo "OUTCOME=gate-timeout ITEM=$ITEM SECS=$GATE_TIMEOUT"; exit 14
    else
      # spec 318: the marker-then-outcome shape of the timeout branch, so the next cycle resumes at verify with no worker bought
      GSTEP=$(grep -E 'failed steps:' "$LOG" 2>/dev/null | tail -1 | cut -c1-200)
      GFIRST=${GSTEP#*failed steps:}; GFIRST=${GFIRST# }; GFIRST=${GFIRST%%,*}
      python3 scripts/autonomy/state.py set "$ITEM" status=needs-review attempts=$((ATTEMPTS+1)) "last=worker-ok; gate-failed: ${GFIRST:-not named in the log}"
      report loop.gate.verdict "item=$ITEM FAILED ${GSTEP:-failing step not named in the log}"
      say "gate FAILED — see $LOG"; echo "OUTCOME=gate-failed ITEM=$ITEM"; exit 10
    fi
  fi
fi

# 6) commit — explicit paths only, neutral identity, no tool ever named
if [ -n "$PATHS" ] && [ "$PATHS" != "-" ]; then
  IFS=';' read -ra P <<<"$PATHS"
  git add -- "${P[@]}" || { say "git add failed"; echo "OUTCOME=add-failed ITEM=$ITEM"; exit 11; }
  # spec 268: the commit carries only the row's paths. A worker edit to any other file stays unstaged, so HEAD would not
  # build (A131, A132). Refuse BEFORE the commit; the tree is left as the worker made it.
  UNLISTED=$(bash scripts/autonomy/unlisted-changes.sh "$PATHS") || {
    git reset -q
    python3 scripts/autonomy/state.py set "$ITEM" status=needs-review "last=unlisted-changes: $(printf '%s\n' "$UNLISTED" | head -5 | paste -sd' ' -)"
    say "NOT COMMITTED: the worker changed files the row does not list: $(printf '%s' "$UNLISTED" | tr '\n' ' ')"
    echo "OUTCOME=unlisted-changes ITEM=$ITEM"; exit 17
  }
  if git diff --cached --quiet; then
    # The item may ALREADY be satisfied: its work can have landed in an earlier commit (a leftover commit is still a
    # commit — measured 2026-10-13: spec 153's whole implementation was in `91dc682`, so every cycle re-dispatched it,
    # the worker changed nothing, and the loop could never finish the item). Close it on exactly the evidence the
    # normal path uses — the item's own criterion plus a green gate — and name the commit that carries the work.
    CARRY=$(git log -1 --format='%h' -- $(printf '%s ' "${P[@]}") 2>/dev/null | tail -1)
    [ -n "$CARRY" ] || CARRY="no commit touches $(echo "$PATHS" | tr ';' ' ')"
    python3 scripts/autonomy/state.py set "$ITEM" status=done attempts=$((ATTEMPTS+1)) "last=satisfied by $CARRY: the criterion passed and fresh-gate is green"
    say "nothing staged — the item is already satisfied by $CARRY (criterion rc=0, gate green): closing it"
    echo "OUTCOME=done-satisfied ITEM=$ITEM CARRY=$CARRY"
    exit 0
  fi
  # 5b) spec 315: the commit is the object under test. The door builds HEAD + the index BEFORE the commit exists, so a
  # tree that does not build is never committed (spec 245's head-builds still runs after, as detection). The index stays
  # staged on a red: the next cycle resumes at verify. attempts is NOT spent — a broken tree is not a worker failure.
  PC=$(timeout "${PRE_COMMIT_TIMEOUT:-600}" bash "${PRE_COMMIT_BUILDS:-scripts/autonomy/pre-commit-builds.sh}" 2>&1); PCRC=$?
  say "pre-commit: $(printf '%s' "$PC" | head -1)"
  if [ "$PCRC" -ne 0 ]; then
    printf '%s\n' "$PC" >>"$LOG"
    if [ "$PCRC" -eq 124 ]; then
      python3 scripts/autonomy/state.py set "$ITEM" status=needs-review "last=worker-ok; precommit-timeout — the check was KILLED, not failed"
      say "pre-commit TIMED OUT after ${PRE_COMMIT_TIMEOUT:-600}s — NOT COMMITTED, the check was killed, not failed"
    else
      PCSYM=$(printf '%s\n' "$PC" | grep -m1 -E '^error' | cut -c1-160)
      python3 scripts/autonomy/state.py set "$ITEM" status=needs-review "last=worker-ok; precommit-broken: ${PCSYM:-$(printf '%s\n' "$PC" | head -1 | cut -c1-160)}"
      say "NOT COMMITTED: the pending tree does not build — the index stays staged"
    fi
    echo "OUTCOME=precommit-broken ITEM=$ITEM"; exit 18
  fi
  # lane-trailer:begin
  # spec 304: the commit's lane is, in order, the lane the tree-changing run recorded on the row (a resume at verify has no fresh log, so the
  # record is what carries it), then the LAST `LANE-GRAPH:` line of the worker log (spec 303), then none: an invented lane is worse than none
  LANE_TRAILER=$(python3 scripts/autonomy/state.py get "${ITEM:-}" 2>/dev/null | python3 -c 'import json,sys; print(json.load(sys.stdin).get("lane",""))' 2>/dev/null)
  case "$LANE_TRAILER" in spec|coder) ;; *) LANE_TRAILER=$(grep -a '^LANE-GRAPH: ' "${WLOG:-}" 2>/dev/null | tail -1 | sed -n 's/^LANE-GRAPH: \(spec\|coder\)$/\1/p') ;; esac
  COMMIT_MSG="$SPEC: automated pass — $TITLE (spec + criteria in specs/$SPEC; gate green)"
  [ -n "$LANE_TRAILER" ] && COMMIT_MSG=$(printf '%s\n\nLane: %s' "$COMMIT_MSG" "$LANE_TRAILER")
  # lane-trailer:end
  git -c user.name="Marcelo Correa" -c user.email="mvcorrea@gmail.com" commit -q \
    -m "$COMMIT_MSG" || { say "commit failed"; exit 11; }
  say "committed: $(git log --oneline -1)"
  # spec 326 R5: the row names its work and its work's commit, written by the step that learns them. Until this
  # existed the sha lived only inside `last`, as prose, and the spec only in the plan's column 2.
  python3 scripts/autonomy/state.py annotate "$ITEM" --spec "specs/$SPEC" --commit "$(git rev-parse HEAD)" \
    >>"$LOG" 2>&1 || say "annotate: the row could not name its commit (see the log) — check-row-truth.sh will say so"
  report loop.commit "item=$ITEM rev=$(git rev-parse --short HEAD)"
  # 6b) spec 245: the gate measured the working tree; this reads the COMMIT. HEAD_BUILDS defaults to the real script.
  HB=$(timeout "${HEAD_BUILDS_TIMEOUT:-1800}" bash "${HEAD_BUILDS:-scripts/autonomy/head-builds.sh}" 2>&1); HBRC=$?
  if [ "$HBRC" -ne 0 ]; then
    HBSYM=$(printf '%s\n' "$HB" | grep -m1 -E '^error' | cut -c1-160)
    python3 scripts/autonomy/state.py set "$ITEM" status=needs-review "last=head-broken after $(git rev-parse --short HEAD): ${HBSYM:-see log}"
    printf '%s\n' "$HB" >>"$LOG"; say "head-builds FAILED: $HB"
    echo "OUTCOME=head-broken ITEM=$ITEM HEAD=$(git rev-parse --short HEAD)"; exit 15
  fi
  say "head-builds: $(printf '%s' "$HB" | head -1)"
fi
python3 scripts/autonomy/state.py set "$ITEM" status=done attempts=$((ATTEMPTS+1)) "last=committed $(git rev-parse --short HEAD)"
say "ITEM $ITEM DONE"; echo "OUTCOME=done ITEM=$ITEM HEAD=$(git rev-parse --short HEAD)"

# 7) chain: the next item starts as soon as the lock is free — the loop must not idle between items
if [ -z "${NO_CHAIN:-}" ]; then
  NEXT=$(python3 scripts/autonomy/state.py pick 2>/dev/null | cut -f1)
  if [ -n "$NEXT" ] && [ "$NEXT" != "NONE" ]; then
    say "chaining: $NEXT will start as soon as this cycle releases the lock"
    setsid nohup bash -c 'sleep 5; bash scripts/autonomy/tick.sh' >>"$LOGD/cycle-last-detached.log" 2>&1 </dev/null &
  else
    say "no chain: the queue has nothing pending"
  fi
fi
