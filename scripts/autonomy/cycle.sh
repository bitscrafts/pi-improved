#!/usr/bin/env bash

# THE LOCK SPANS THE WHOLE CYCLE (2026-10-15). The launcher exits immediately, so a lock taken by the launcher is
# released the moment the cycle starts: that is how four cycles ran at once and their builds queued on cargo.
if [ -z "${PII_CYCLE_LOCKED:-}" ]; then
  export PII_CYCLE_LOCKED=1
  # not `exec flock ... || ...`: exec replaces this shell, so the || never ran and a held lock exited 1, silently
  flock -n -E 75 "${PII_LOCK:-/projects/pi-improved/_tmp/autonomy/lock}" bash "$0" "$@"; rc=$?
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
PICK=$(python3 scripts/autonomy/state.py pick)
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
  BRIEF="_docs/05-operations/QUEUE-BRIEF-$(date -u +%Y-%m-%dT%H).md"
  mkdir -p _docs/05-operations
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
  # Keep the diff as a patch, commit the paths as a named leftover, and carry on: nothing is lost, and the
  # commit is reversible.
  # Before treating a dirty tree as a leftover, look for a session to RESUME: a worker cut off mid-flight leaves
  # its session id in its own log even when the dispatcher never got to write it (measured 2026-10-13).
  RECOVER=$(bash scripts/autonomy/session.sh recover "$(printf '%s' "$SPEC" | sed -E 's/^([0-9]+).*/\1/')" 2>/dev/null || true)
  if [ -n "$RECOVER" ]; then
    python3 scripts/autonomy/state.py set "$ITEM" status=needs-review "session=$RECOVER" "last=resumed-from-worker-log"
    say "dirty tree WITH a session recovered from the worker's log ($RECOVER): resuming, not restarting"
    RSID="$RECOVER"; ARGS=(--resume "$RECOVER")
  else
  LEFT=$STATEDIR/leftovers; mkdir -p "$LEFT"
  git diff >"$LEFT/$CID-tracked.patch" 2>/dev/null || true
  git status --porcelain | awk '{print $2}' >"$LEFT/$CID-paths.txt"
  say "DIRTY TREE with no session: keeping the leftover as $LEFT/$CID-tracked.patch and committing it as a named leftover"
  mapfile -t LP < <(git status --porcelain | awk '{print $2}')
  [ "${#LP[@]}" -gt 0 ] && git add -- "${LP[@]}" 2>>"$LOG" || true
  if git diff --cached --quiet; then say "nothing stageable in the leftover"; else
    git -c user.name="Marcelo Correa" -c user.email="mvcorrea@gmail.com" commit -q \
      -m "leftover from the previous item, kept for review (cycle $CID; the diff is also at $LEFT/$CID-tracked.patch)" 2>>"$LOG" || true
    say "leftover committed: $(git log --oneline -1)"
  fi
  bash scripts/autonomy/control.sh ask "found a leftover from the previous item and committed it for review (cycle $CID). Nothing was lost; check the message if it looks wrong." >/dev/null 2>&1 || true
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
      LASTCYCLE=$(ls -t "$LOGD"/cycle-*.log 2>/dev/null | head -1)
      { echo "# Corrective brief — $ITEM ($SPEC). The previous attempt ended: $PRV"; echo
        echo "Your earlier pass on this spec did not survive the orchestrator's check. Evidence:"
        echo '```'
        grep -E 'failed steps:|failing tests:|panicked at|FAIL: |^OUTCOME=' "$LASTCYCLE" 2>/dev/null | head -20
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
if [ -n "$SKIP_DISPATCH" ]; then
  NEWSID="$RSID"
else
  if [ "${ATTEMPTS:-0}" -ge 3 ]; then
    say "attempts=$ATTEMPTS reached the cap of 3 — PARKING for a person, not dispatching again"
    python3 scripts/autonomy/state.py set "$ITEM" status=parked attempts="$ATTEMPTS" "last=parked after $ATTEMPTS attempts: the same mechanical failure repeated, so a fourth dispatch would spend without new information"
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
  if [ "${PII_WORKER:-pi}" = "claude" ]; then
    ( ALLOW_DIRTY=1 bash scripts/run-batchC-claude.sh "$N" "${ARGS[@]}" "${BRIEFARG[@]}" ) >>"$LOG" 2>&1
  else
    ( ALLOW_DIRTY=1 bash scripts/run-batchC-pi.sh "$N" "${BRIEFARG[@]}" ) >>"$LOG" 2>&1
  fi
      ;;
  esac
  WRC=$?
  NEWSID=$(awk '{print $1}' _tmp/claude-batchC/session-"$N"-*.txt 2>/dev/null | tail -1)
  say "worker rc=$WRC session=${NEWSID:-none}"
  report loop.dispatch "item=$ITEM session=${NEWSID:-none} rc=$WRC"
  # The worker may have been refused by the account's WINDOW rather than stopped by its work. The 1-turn probe can
  # say OPEN while the worker is refused (measured 2026-10-13), so read the worker's OWN log for the CLI's words and
  # honour the reset it states — never blind-retry, never shrink scope (the project's oldest operational rule).
  WLOG=$(ls -t _tmp/claude-batchC/"$N"-*.log 2>/dev/null | head -1)
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
  python3 scripts/autonomy/state.py set "$ITEM" status=needs-review "session=${NEWSID:-}" last=worker-rc=$WRC
  [ $WRC -ne 0 ] && { say "worker did not finish cleanly — leaving the WIP for the next cycle to resume"; echo "OUTCOME=worker-rc-$WRC ITEM=$ITEM SESSION=${NEWSID:-}"; exit 8; }
  # QUOTA IS NOT A FAILURE (user rule 2026-10-19: "eventually it will exaust, let it run to the end then put it again on
  # the queue"). A spend/rate limit must not park the row for a person: capture the reset the CLI itself printed, put the
  # row BACK on the queue with its attempts and session intact, and let the next pass resume it.
  if [ -n "${WLOG:-}" ] && grep -qiE "you've hit your (session|weekly|monthly|usage) limit|\"rate_limit_error\"|\"type\":\"rate_limit\"|resets [0-9]{1,2}:[0-9]{2}" "$WLOG" 2>/dev/null; then
    RESET=$(grep -oiE '(resets? (at )?[^.,;]{0,40})' "$WLOG" 2>/dev/null | head -1)
    python3 scripts/autonomy/state.py set "$ITEM" status=pending attempts="$ATTEMPTS" \
      "last=requeued: provider limit hit (${RESET:-reset time not printed}) — resume the same session, do not restart" >/dev/null 2>&1
    say "QUOTA: $ITEM back on the queue — ${RESET:-reset unknown}"
    echo "OUTCOME=requeued-quota ITEM=$ITEM SESSION=$RECOVER"; exit 0
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
  [ $((ATTEMPTS+1)) -ge 3 ] && { python3 scripts/autonomy/state.py set "$ITEM" status=parked "last=3-strikes"; report loop.park "item=$ITEM 3-strikes"; }
  report loop.verify "item=$ITEM FAILED: $VFAIL"
  say "VERIFY FAILED: $VFAIL"; echo "OUTCOME=verify-failed ITEM=$ITEM CMD=$VFAIL"; exit 9
fi

report loop.verify "item=$ITEM ok"

# 5) the gate — a green verify is not a green gate
# Only an explicit 0 / - / none skips the gate: rows A22-A24 named "quality-gate.sh" here, failed an `= 1` test, and
# were committed with no fresh-gate under a message that said "gate green" (075fee9). Fail toward gating.
case "${GATE:-1}" in 0|-|none) GATE_ON= ;; *) GATE_ON=1 ;; esac
if [ -n "$GATE_ON" ]; then
  FREE=$(df -m --output=avail /projects | tail -1 | tr -d ' ')
  if [ "$FREE" -lt 20000 ]; then
    # A skipped gate must NOT fall through to the commit: an ungated commit is worse than a parked item.
    python3 scripts/autonomy/state.py set "$ITEM" status=needs-review attempts=$((ATTEMPTS+1)) last=gate-skipped-disk
    say "gate SKIPPED: only ${FREE}M free (needs 20000M) — the item is NOT committed without a gate"
    echo "OUTCOME=gate-skipped-disk ITEM=$ITEM FREE_MB=$FREE"
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
    ( export PATH="/usr/local/cargo/bin:$PATH"; CARGO_TARGET_DIR=/projects/pi-improved/_tmp/gate-target FRESH_GATE_MIN_FREE_MB=12000 timeout "$GATE_TIMEOUT" bash quality-gate.sh --routine ) >>"$LOG" 2>&1
    GTRC=$?
    if [ "$GTRC" -eq 0 ]; then
      python3 scripts/autonomy/state.py set "$ITEM" last=gated; say "fresh-gate PASS (phase marked: gated)"
      report loop.gate.verdict "item=$ITEM pass"
    elif [ "$GTRC" -eq 124 ]; then
      # A KILLED gate is not a verdict. Decided 2026-10-14 (DECISION-fresh-worktrees-and-a-resumable-gate-build.md
      # §2): a timeout gets its own outcome, because "FAILED" must mean the tree is wrong, and a gate that says
      # FAILED while the tree is fine is the same lie as a check that passes while its path is broken. Attempts
      # are NOT incremented and the marker keeps the next cycle at VERIFY, so a slow host costs a retry — never a
      # worker re-dispatch (which would re-bill a paid run) and never a park dressed as a code failure.
      python3 scripts/autonomy/state.py set "$ITEM" status=needs-review "last=worker-ok; gate-timeout after ${GATE_TIMEOUT}s — the gate was KILLED, not failed; retried at verify"
      say "fresh-gate TIMED OUT after ${GATE_TIMEOUT}s — NOT MEASURED, not a failure; the item is retried at verify"
      echo "OUTCOME=gate-timeout ITEM=$ITEM SECS=$GATE_TIMEOUT"; exit 14
    else
      python3 scripts/autonomy/state.py set "$ITEM" status=needs-review attempts=$((ATTEMPTS+1)) last=gate-failed
      GSTEP=$(grep -E 'failed steps:' "$LOG" 2>/dev/null | tail -1 | cut -c1-200)
      report loop.gate.verdict "item=$ITEM FAILED ${GSTEP:-failing step not named in the log}"
      say "fresh-gate FAILED — see $LOG"; echo "OUTCOME=gate-failed ITEM=$ITEM"; exit 10
    fi
  fi
fi

# 6) commit — explicit paths only, neutral identity, no tool ever named
if [ -n "$PATHS" ] && [ "$PATHS" != "-" ]; then
  IFS=';' read -ra P <<<"$PATHS"
  git add -- "${P[@]}" || { say "git add failed"; echo "OUTCOME=add-failed ITEM=$ITEM"; exit 11; }
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
  git -c user.name="Marcelo Correa" -c user.email="mvcorrea@gmail.com" commit -q \
    -m "$SPEC: automated pass — $TITLE (spec + criteria in specs/$SPEC; gate green)" || { say "commit failed"; exit 11; }
  say "committed: $(git log --oneline -1)"
  report loop.commit "item=$ITEM rev=$(git rev-parse --short HEAD)"
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
