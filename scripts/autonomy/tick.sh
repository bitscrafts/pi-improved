#!/usr/bin/env bash
# tick.sh — one scheduled tick, no LLM involved, and QUIET unless something happened.
#
# This is the engine's heartbeat: it starts exactly one cycle, or reports the one that is running, and prints
# at most two lines — which the scheduler delivers verbatim to the phone. Empty output sends nothing.
#
# Why no agent: a cron agent inherits the session's model/provider and dies on a 90 s API timeout before it can
# run anything (measured 2026-10-10, and again 2026-10-11 when the first tick of the autonomy job fell from
# gemini to an nvidia fallback and timed out). A deterministic script cannot be starved by a provider. The
# judgement (reddening a mutation by hand, judging a live proof, authoring the next spec) is a separate,
# best-effort supervisor job — if that one dies, this loop keeps going.
#
# Quiet rules: a cycle in flight prints nothing; a milestone (a commit, a park, a closed window) prints once,
# and printing is de-duplicated against _tmp/autonomy/last-reported so the same fact is not re-sent every tick.
. "$(dirname "$(readlink -f "$0")")/env.sh"   # the loop needs PATH set inside the script
set -uo pipefail
cd /projects/pi-improved || { echo "pii autonomy: cannot cd to the project"; exit 1; }
ST=_tmp/autonomy; LOGD=$ST/logs; mkdir -p "$LOGD"
REPORTED=${PII_REPORTED:-$ST/last-reported}

say_once() { # say_once <key> <line...>   — print only when <key> differs from the last reported key
  local key="$1"; shift
  [ "$(cat "$REPORTED" 2>/dev/null)" = "$key" ] && return 0
  printf '%s' "$key" >"$REPORTED"
  printf '%s\n' "$@"
}

# 0) the gateway delivers this loop's messages and runs the other scheduler. It died on 2026-10-11 and nothing
#    revived it: no systemd in this container, and its own log said it exited "so systemd Restart=on-failure can
#    revive the gateway". For two days nothing reached the phone because of it. Revive it here, but only when no
#    paid worker is in flight (HARD RULE 0f).
# the check must not fire when the tool is not on PATH (a test's minimal environment made the tick believe the
# gateway was down and exit before it could report anything — measured 2026-10-13)
# A SINGLE probe is a coin toss under load: measured 2026-10-19, 2 of 25 probes failed while a cargo test ran,
# and every false negative cost the whole beat — no cycle was dispatched from 09:12Z to 10:15Z while the
# gateway was up the entire time. Probe three times, two seconds apart, before believing it is down.
gw_up() {
  local i
  for i in 1 2 3; do
    hermes gateway status 2>/dev/null | grep -q 'Gateway is running' && return 0
    [ "$i" -lt 3 ] && sleep 2
  done
  return 1
}
if command -v hermes >/dev/null 2>&1 && ! gw_up; then
  if ps -eo cmd | grep -qE '[c]laude -p|[r]un-batchC'; then
    say_once "gw-dead-worker" "pii autonomy · the gateway is down but a worker is in flight — not restarting it now (0f)"
  else
    setsid nohup hermes gateway run >/root/.hermes/logs/gateway-restart.log 2>&1 </dev/null &
    sleep 12
    if gw_up; then
      say_once "gw-up" "pii autonomy · the gateway was down and is running again (it delivers these messages)"
    else
      say_once "gw-still-down" "pii autonomy · the gateway is down and would not start — messages cannot be delivered"
    fi
  fi
  exit 0
fi

# 0b) an incident is opened from our own records (spec 150): N identical outcomes within 30 min, no model. One
#     phone line per NEW incident; `--new` prints nothing when none is new, so this is silent in the normal case.
#     Needs the built binary; a missing one is skipped, never compiled here (a tick must not start a build).
PII_BIN=${PII_BIN:-target/debug/pii}
if [ -x "$PII_BIN" ]; then
  "$PII_BIN" incidents scan --root "$LOGD" --new 2>/dev/null | while read -r _ FP COUNT FIRST _; do
    echo "pii autonomy · INCIDENT $FP — ${COUNT#count=} times since ${FIRST#first_seen=} (see $LOGD/incidents.jsonl)"
  done
fi

# 0c) paused by the operator (Discord: `control.sh pause`) — say it once, start nothing.
#     REPORTED HERE, before the in-flight and window branches, because both of those exit 0 and a tick that exits
#     early never told the operator that the switch they flipped is on. That is measured, twice: the selftest row
#     "paused -> the tick says so (once)" failed on 2026-10-14 (gateway down) and again on 2026-10-17 (a cycle in
#     flight), 32 passed / 2 failed, for no reason other than branch order. A switch nobody hears is not a control.
if grep -q PAUSED "${PII_CONTROL:-$ST/control}" 2>/dev/null; then
  say_once "paused:$(sed -n 1p "${PII_CONTROL:-$ST/control}")" "pii autonomy · PAUSED — $(sed -n 1p "${PII_CONTROL:-$ST/control}")"
  # A cycle already in flight is NOT stopped by the switch (measured 2026-10-15: a paid worker ran 83 min into a
  # pause). Fall through to branch 1 so its progress still reaches the phone; branch 1 exits without dispatching.
  # With no cycle in flight, stop here: the pause must still buy nothing.
  if flock -n "${PII_LOCK:-$ST/lock}" true 2>/dev/null; then exit 0; fi
fi

# 0d) an open question must reach the phone (the user's instruction: "qualquer duvida, pelo discord"), and it must
#     reach it even while a cycle runs — so this sits above the in-flight branch, whose exit 0 used to swallow it
#     (the selftest row "an open question is carried to the phone" failed with 32 passed / 2 failed on 2026-10-17
#     for that reason). Keyed by the question list itself, so the same question is not re-sent every tick.
Q="${PII_QUESTIONS:-$ST/questions.md}"
if [ -f "$Q" ] && grep -q '^- \[ \]' "$Q"; then
  KEY="q:$(grep '^- \[ \]' "$Q" | md5sum | cut -c1-12)"
  OPEN=$(grep -c '^- \[ \]' "$Q")
  say_once "$KEY" "pii autonomy · $OPEN open question(s) for you:" "$(grep '^- \[ \]' "$Q" | head -3 | sed 's/^- \[ \] //' | cut -c1-160)"
fi

# 1) a cycle is running — silence is the normal state; only a stuck worker is worth a line
if ! flock -n "${PII_LOCK:-$ST/lock}" true 2>/dev/null; then
  AGE=$(( $(date +%s) - $(stat -c %Y "${PII_LOCK:-$ST/lock}" 2>/dev/null || date +%s) ))
  BUCKET=$(( AGE / 1800 ))
  if [ "$AGE" -gt 5400 ]; then
    say_once "stuck:$BUCKET" "pii autonomy · a cycle has been in flight for $((AGE/60)) min" "check: tail -30 $(ls -t "$LOGD"/cycle-*.log | head -1)"
  else
    # PROGRESS, not only an alarm: from the outside a working worker and a stall look identical, and the user asked
    # "looks stopped?" with a worker 9 minutes into an honest job. One line per 30-minute bucket, keyed so it is sent
    # once per bucket and not every tick.
    ITEM=$(grep -o 'item=[^ ]*' "$(ls -t "$LOGD"/cycle-*.log 2>/dev/null | head -1)" 2>/dev/null | tail -1 | cut -d= -f2)
    WLOG=$(ls -t _tmp/claude-batchC/*.log 2>/dev/null | head -1)
    SZ=$(du -m "$WLOG" 2>/dev/null | cut -f1)
    say_once "flying:$BUCKET:$ITEM" "pii autonomy · ${ITEM:-a cycle} in flight $((AGE/60)) min (worker stream ${SZ:-0} MB) — working, not stuck"
  fi
  exit 0
fi

# 1b) the account's window: if the CLI itself said when it reopens, do not probe before then
if bash scripts/autonomy/window.sh check 2>/dev/null; then
  # the marker is a THROTTLE: it stops a probe storm, it does not block for ever. A live probe has the last word,
  # so after 30 min of believing the marker, probe anyway (measured 2026-10-13: a stale limit line parked the loop
  # while the window was open, and the user had to say so).
  AGE=$(( $(date -u +%s) - $(stat -c %Y _tmp/autonomy/window-closed-until 2>/dev/null || echo 0) ))
  if [ "$AGE" -lt 1800 ]; then
    say_once "closed:$(cat _tmp/autonomy/window-closed-until)" "pii autonomy · the account window is closed ($(bash scripts/autonomy/window.sh show)) — nothing spent"
    exit 0
  fi
  echo "pii autonomy · the marker is $(($AGE/60)) min old — probing again (the marker throttles, it does not block)"
fi

# 2) paused is reported at 0c, above the branches that exit early. Kept as a note, not a branch: a switch checked
#    after two exits is a switch that goes unheard exactly when the loop is busy.

# 3) the last cycle's outcome
LASTLOG=$(ls -t "$LOGD"/cycle-*.log 2>/dev/null | head -1)
OUT=$(grep -o 'OUTCOME=[^ ]*' "$LASTLOG" 2>/dev/null | tail -1 | cut -d= -f2)
ITEM=$(grep -o 'ITEM=[^ ]*' "$LASTLOG" 2>/dev/null | tail -1 | cut -d= -f2)
HEAD=$(git log --oneline -1 2>/dev/null | cut -c1-60)
NEXT=$(python3 scripts/autonomy/state.py pick 2>/dev/null | cut -f1,4)

case "$OUT" in
  done)          say_once "done:$ITEM:$HEAD" "pii autonomy · $ITEM done" "$HEAD" ;;
  verify-failed|gate-failed|dirty-tree|bad-plan)
                 say_once "bad:$ITEM:$OUT" "pii autonomy · $ITEM needs a hand ($OUT)" "log: ${LASTLOG##*/}" ;;
  worker-rc-*)   # resumed only when the queue's own pick names this item (spec 313)
                 if [ -n "$ITEM" ] && [ -n "${NEXT:-}" ] && [ "${NEXT%%$'\t'*}" = "$ITEM" ]; then
                   say_once "resumed:$ITEM:$OUT" "pii autonomy · $ITEM resuming with its session ($OUT)" "log: ${LASTLOG##*/}"
                 else
                   say_once "bad:$ITEM:$OUT" "pii autonomy · $ITEM stopped early with no session to resume — needs a hand ($OUT)" "log: ${LASTLOG##*/}"
                 fi ;;
  precommit-*)   # a refused commit (spec 316): resumed only when the queue's own pick names this item; neither line says the item is done
                 if [ -n "$ITEM" ] && [ -n "${NEXT:-}" ] && [ "${NEXT%%$'\t'*}" = "$ITEM" ]; then
                   say_once "regate:$ITEM:$OUT" "pii autonomy · $ITEM re-gating with its session (the commit was refused: the pending tree did not build)" "log: ${LASTLOG##*/}"
                 else
                   say_once "bad:$ITEM:$OUT" "pii autonomy · $ITEM's commit was refused and no cycle can resume it — needs a hand ($OUT)" "log: ${LASTLOG##*/}"
                 fi ;;
  window-closed) say_once "closed:$(cut -c1-20 <<<"$(grep -o 'resets [^ ]* [^ ]*' "$LASTLOG" 2>/dev/null | tail -1)")" \
                   "pii autonomy · the Anthropic window is closed — nothing spent, the queue is untouched" ;;
esac

# 2b) an open question is reported at 0d, above the branches that exit early (same reason as 0c).

# 2b-2) the review becomes queue rows (spec 167, doc 066): the importer copies ```queue blocks and refuses what it
#       cannot verify; it takes the cycle lock itself (exit 75 = held, nothing read). It commits only the files it wrote.
if [ -z "${TICK_DRY:-}" ]; then
  python3 scripts/autonomy/plan_import.py import _docs/06-research/*.md 2>>"$LOGD/plan-import.log" | while IFS=$'\t' read -r _ BID RID SPECF; do
    git add -- scripts/autonomy/plan.tsv scripts/autonomy/plan.provenance.tsv "$SPECF" &&
      git -c user.name="Marcelo Correa" -c user.email="mvcorrea@lnx" commit -q -m "plan-import: $BID → $RID" -- scripts/autonomy/plan.tsv scripts/autonomy/plan.provenance.tsv "$SPECF"
  done
fi

# 2c) TICK_DRY=1: say what would start, and start nothing (the selftest uses this; a test must never buy work)
if [ -n "${TICK_DRY:-}" ]; then
  echo "pii autonomy · dry: would start ${NEXT:-the next pending item}"
  exit 0
fi

# 3) start exactly one cycle, detached
setsid nohup bash scripts/autonomy/cycle.sh >>"$LOGD/cycle-last-detached.log" 2>&1 </dev/null &
sleep 2
exit 0
