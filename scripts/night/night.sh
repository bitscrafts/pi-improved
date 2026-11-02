#!/usr/bin/env bash
# night.sh — the night's loop: FILL the queue, let the standing dispatcher DRAIN it, run Opus at the end of each
# drain, refill, repeat, until the deadline (07:00 local = 10:00Z by default).
#
# WHY A TICK AND NOT A LONG-LIVED PROCESS. A shell that sleeps until 07:00 dies with the gateway, with the
# container, or with an OOM kill, and leaves no record of how far it got. One bounded step per invocation is
# resumable by construction: the state file says which tranche is seeded and how many reviews ran, and the next
# tick continues from there. The queue itself is drained by the ALREADY SCHEDULED dispatcher
# (`pii-drain.sh`, every 5 min), because a second dispatcher is how the loop deadlocked before two of every three
# runs logged OUTCOME=locked.
#
# Usage:
#   night.sh [--tick]            one step: drain-wait | review | refill | done
#   night.sh --seed <tranche>    append a tranche's rows to plan.tsv (a|b)
#   night.sh --status            what the state file says, and whether the queue is empty
#   night.sh --until <ISO8601>   override the deadline (default <today>T10:00:00Z)
#   night.sh --dry-run           say what the tick would do, change nothing
#
# Env: NIGHT_MAX_OPUS (default 2), NIGHT_DIR (default _tmp/night), NIGHT_OPUS_WORKDIR.
set -uo pipefail
cd /projects/pi-improved || exit 1
. scripts/autonomy/env.sh 2>/dev/null

DIR=${NIGHT_DIR:-_tmp/night}
STATE=$DIR/state.json
LOG=$DIR/night.log
LOCK=$DIR/.lock
PLAN=${NIGHT_PLAN:-scripts/autonomy/plan.tsv}
MAX_OPUS=${NIGHT_MAX_OPUS:-2}
NOW_Z=$(date -u +%Y-%m-%dT%H:%M:%SZ)
UNTIL=${NIGHT_UNTIL:-$(date -u +%Y-%m-%d)T10:00:00Z}
MODE=${1:---tick}

mkdir -p "$DIR"
say(){ printf '%s %s\n' "$(date -u +%H:%M:%SZ)" "$*" >>"$LOG"; printf '%s\n' "$*"; }

# state: {"tranche":"none|a|b","opus_runs":0,"seeded":[],"done":false}
[ -f "$STATE" ] || printf '%s\n' '{"tranche":"none","opus_runs":0,"seeded":[],"done":false}' >"$STATE"
sf(){ python3 -c "import json,sys; d=json.load(open('$STATE')); print(d.get('$1'))"; }

past_deadline(){ [ "$(date -u +%s)" -ge "$(date -u -d "$UNTIL" +%s)" ]; }

queue_empty(){ python3 scripts/autonomy/state.py pick 2>/dev/null | grep -qx 'NONE'; }

# open_rows: rows of THIS night that are still being worked or waiting for work. A review must not run while one
# exists — the operator's rule, 2026-11-02: "it only will go to the next opus review after all gaps were closed".
# A row that is parked is not open: it is a recorded failure with its evidence, and the review is exactly what
# should look at it. A row that is skipped is not open either, but it is also not closed, and it is named in the
# log line so the morning report can say so.
open_rows(){
  python3 scripts/autonomy/state.py list 2>/dev/null \
    | awk -F'\t' '$1>=335 && $1<=340 && ($4=="pending" || $4=="running" || $4=="needs-review") {print $1}' \
    | tr '\n' ' '
}
# A worker is a cycle or a batch runner, matched on the INTERPRETER + script, never on a bare substring: a shell
# whose own command line mentions the pattern (a grep, a test, an editor) would otherwise look like a running worker
# and the whole night would sit still. Measured 2026-11-02: that is exactly what happened.
worker_busy(){ pgrep -f 'bash .*(run-batchC-pii\.sh|scripts/autonomy/cycle\.sh)' >/dev/null 2>&1; }

seed(){
  local t=$1 file=${NIGHT_TRANCHES:-scripts/night}/tranche-$1.tsv n=0
  [ -f "$file" ] || { say "night: no tranche $t to seed"; return 1; }
  grep -q '^#' "$PLAN" || true
  while IFS= read -r row; do
    case "$row" in ''|'#'*) continue ;; esac
    local id=${row%%$'\t'*}
    if grep -q "^$id"$'\t' "$PLAN"; then say "night: row $id already in the plan, skipped"; continue; fi
    printf '%s\n' "$row" >>"$PLAN"
    n=$((n+1))
  done <"$file"
  say "night: seeded tranche $t ($n row(s) appended to $PLAN)"
  python3 - "$STATE" "$t" <<'PY'
import json,sys
p,t=sys.argv[1],sys.argv[2]
d=json.load(open(p)); d["tranche"]=t; json.dump(d,open(p,"w"))
PY
  return 0
}

write_brief(){
  local out=$DIR/brief-$1.md
  {
    printf '# NIGHT BRIEF — pii, %s (automatic pass; plan mode, read-only)\n\n' "$NOW_Z"
    printf 'You are reviewing an autonomous night of work on **pii** (`/projects/pi-improved`). This brief was written by a script, not a person: treat every number as a claim to verify with a command.\n\n'
    printf '## Where the tree is\n\n```\nHEAD      %s\ntree      %s changed path(s)\nqueue     %s\ngates     %s\n```\n\n' \
      "$(git log --format='%h %s' -1)" "$(git status --porcelain | wc -l | tr -d ' ')" \
      "$(python3 scripts/autonomy/state.py pick 2>/dev/null | head -1)" \
      "$(bash scripts/checks/gates.sh 2>&1 | tail -1)"
    printf '## What this night was for\n\n'
    printf 'The previous audit (`_docs/06-research/opus-status-and-path-20261102.md`) ranked a single next action first: a\nread-only host conformance probe, because the gates run in the container and production runs on the host. Tonight the\nqueue was filled from that audit'"'"'s seven-day list and drained by the standing dispatcher:\n\n'
    for s in specs/335-*.md specs/336-*.md specs/337-*.md specs/338-*.md specs/339-*.md specs/340-*.md; do
      [ -f "$s" ] && printf -- '- `%s` — %s\n' "$s" "$(sed -n '1s/^# Spec: //p' "$s")"
    done
    printf '\nTheir rows and outcomes:\n\n```\n%s\n```\n\n' "$(grep -v '^#' "$PLAN" | awk -F'\t' '$1>=335 && $1<=340 {print $1"\t"$4}' | head -12)"
    printf 'Row state:\n\n```\n%s\n```\n\n' "$(python3 scripts/autonomy/state.py list 2>/dev/null | awk '$1>=335 && $1<=340' | head -12)"
    printf '## What I want from you\n\n'
    printf '1. **Which of tonight'"'"'s rows is real work and which is a check that passes while the path is broken?** For each row\n   that claims done, name the command you would run and the case its own test cannot see.\n'
    printf '2. **Is the queue still the right queue?** Re-rank what is left of the seven-day plan and say what should be dropped.\n'
    printf '3. **What did this night make worse?** A commit that changes behaviour without a gate row, a doc that now lies, a\n   check that can no longer fail.\n'
    printf '4. **The next three actions**, each with the observation that would prove it worked and the one that would prove it\n   did not.\n\n'
    printf 'End with the single next action, its cost, and its gate. Correct any premise of this brief that is wrong: a verified\ncontradiction is worth more than agreement.\n'
  } >"$out"
  printf '%s\n' "$out"
}

review(){
  local brief res
  brief=$(write_brief "$(date -u +%Y%m%dT%H%M%SZ)")
  say "night: Opus review starting (brief: $brief)"
  # the workdir stamp must not contain COLONS: a directory named …T07:12:54Z made the filer's `cd` fail, and it
  # still exited 0, so the run reported "filed" with nothing filed.
  local stamp; stamp=$(date -u +%Y%m%dT%H%M%SZ)
  res=$(bash /root/.hermes/skills/research/opus-review/scripts/run-opus-review.sh \
        /projects/pi-improved "$brief" "${NIGHT_OPUS_WORKDIR:-_tmp/night/opus-$stamp}" 2>&1 | tail -3)
  say "night: opus -> $(printf '%s' "$res" | tr '\n' ' ' | cut -c1-200)"
  local plan dest
  plan=$(ls -t _tmp/night/opus-*/opus-review-plan-*.md 2>/dev/null | head -1)
  if [ -z "$plan" ]; then
    say "night: NO plan file found — the review produced nothing this tick"; return 1
  fi
  dest="_docs/06-research/night-review-$stamp.md"
  bash /root/.hermes/skills/research/opus-review/scripts/file-opus-review.sh \
    /projects/pi-improved "$plan" "$dest" >/dev/null 2>&1 || true
  # the exit code of the filer is not the proof: the FILE is.
  if [ -s "$dest" ]; then
    say "night: review filed at $dest ($(wc -c <"$dest" | tr -d ' ') bytes)"
  else
    say "night: the filer reported success but $dest does not exist — filing FAILED"
  fi
  python3 - "$STATE" <<'PY'
import json,sys
p=sys.argv[1]; d=json.load(open(p)); d["opus_runs"]=int(d.get("opus_runs",0))+1; json.dump(d,open(p,"w"))
PY
}

summary(){
  {
    printf '# The night, %s → %s\n\n' "$(python3 -c "import json;print(json.load(open('$STATE')).get('started','?'))" 2>/dev/null)" "$NOW_Z"
    printf 'Tranches seeded: %s · Opus reviews: %s\n\n' "$(sf tranche)" "$(sf opus_runs)"
    printf '## Rows\n\n```\n%s\n```\n\n' "$(python3 scripts/autonomy/state.py list 2>/dev/null | awk '$1>=335' | head -20)"
    printf '## Commits made tonight\n\n```\n%s\n```\n' "$(git log --since="$UNTIL" --format='%h %ad %s' --date=format:'%H:%M' | head -20)"
  } >"$DIR/SUMMARY.md"
  say "night: summary written to $DIR/SUMMARY.md"
}

case "$MODE" in
  --status)
    echo "state     : $(cat "$STATE")"
    echo "deadline  : $UNTIL (past: $(past_deadline && echo yes || echo no))"
    echo "queue     : $(python3 scripts/autonomy/state.py pick 2>/dev/null | head -1)"
    echo "worker    : $(worker_busy && echo busy || echo idle)"
    exit 0 ;;
  --seed) seed "${2:?--seed needs a tranche name (a|b)}"; exit $? ;;
  --until) UNTIL=$2; say "night: deadline $UNTIL"; exit 0 ;;
  --review-now)
    # The review step is the one part of this loop that has to call a paid model, so it must be runnable on its own
    # and on a clean tree — otherwise its first execution is at 07:00 with nobody awake. Guarded exactly as the
    # night guards it: one review at a time, and never while a worker is editing the tree.
    if pgrep -f 'night\.sh --tick' >/dev/null 2>&1 && [ "${2:-}" != "--allow-concurrent" ]; then
      say "night: another fill/review step is running — refusing to start a second"; exit 3
    fi
    if pgrep -f 'bash .*(run-batchC-pii\.sh|scripts/autonomy/cycle\.sh)' >/dev/null 2>&1 && [ "${2:-}" != "--allow-dirty" ]; then
      say "night: a worker is editing the tree — a review now would judge half-written work; refusing"; exit 3
    fi
    [ -z "$(git status --porcelain)" ] || { say "night: the tree is dirty; a review judges the tree, refusing"; exit 3; }
    review
    exit $? ;;
  --dry-run)
    echo "would: past_deadline=$(past_deadline && echo yes || echo no) queue_empty=$(queue_empty && echo yes || echo no) worker=$(worker_busy && echo busy || echo idle) tranche=$(sf tranche) opus_runs=$(sf opus_runs)"
    exit 0 ;;
esac

# ---- the tick ---------------------------------------------------------------------------------------------
if [ "$(sf done)" = "True" ]; then say "night: already done (see $DIR/SUMMARY.md)"; exit 0; fi
if past_deadline; then
  summary; python3 - "$STATE" <<'PY'
import json,sys
p=sys.argv[1]; d=json.load(open(p)); d["done"]=True; json.dump(d,open(p,"w"))
PY
  say "night: deadline $UNTIL reached — stopping"; exit 0
fi
if ! mkdir "$LOCK" 2>/dev/null; then say "night: another tick holds the lock — nothing done"; exit 0; fi
trap 'rmdir "$LOCK" 2>/dev/null' EXIT

if worker_busy; then say "night: a worker is in flight — the drain is running, nothing to do"; exit 0; fi
if ! queue_empty; then
  say "night: queue has work ($(python3 scripts/autonomy/state.py pick 2>/dev/null | head -1 | cut -c1-60)) — the dispatcher owns it"
  exit 0
fi
case "$(sf tranche)" in
  none) seed a || { say "night: tranche a is missing — nothing to fill with"; exit 1; } ;;
  a)
    open=$(open_rows)
    if [ -n "$open" ]; then say "night: NO review yet — row(s) $open still open in tranche a"; exit 0; fi
    if [ "$(sf opus_runs)" -lt "$MAX_OPUS" ]; then review; fi
    if [ -f "${NIGHT_TRANCHES:-scripts/night}/tranche-b.tsv" ]; then seed b; else say "night: no tranche b; the night is done"; summary; fi ;;
  b)
    open=$(open_rows)
    if [ -n "$open" ]; then say "night: NO review yet — row(s) $open still open in tranche b"; exit 0; fi
    if [ "$(sf opus_runs)" -lt "$MAX_OPUS" ]; then review; fi
    summary
    python3 - "$STATE" <<'PY'
import json,sys
p=sys.argv[1]; d=json.load(open(p)); d["done"]=True; json.dump(d,open(p,"w"))
PY
    say "night: both tranches drained and reviewed — done" ;;
esac
