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
if command -v hermes >/dev/null 2>&1 && ! hermes gateway status 2>/dev/null | grep -q 'Gateway is running'; then
  if ps -eo cmd | grep -qE '[c]laude -p|[r]un-batchC'; then
    say_once "gw-dead-worker" "pii autonomy · the gateway is down but a worker is in flight — not restarting it now (0f)"
  else
    setsid nohup hermes gateway run >/root/.hermes/logs/gateway-restart.log 2>&1 </dev/null &
    sleep 12
    if hermes gateway status 2>/dev/null | grep -q 'Gateway is running'; then
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

# 2) paused by the operator (Discord: `control.sh pause`) — say it once, start nothing
if grep -q PAUSED "${PII_CONTROL:-$ST/control}" 2>/dev/null; then
  say_once "paused:$(sed -n 1p "${PII_CONTROL:-$ST/control}")" "pii autonomy · PAUSED — $(sed -n 1p "${PII_CONTROL:-$ST/control}")"
  exit 0
fi

# 3) the last cycle's outcome
LASTLOG=$(ls -t "$LOGD"/cycle-*.log 2>/dev/null | head -1)
OUT=$(grep -o 'OUTCOME=[^ ]*' "$LASTLOG" 2>/dev/null | tail -1 | cut -d= -f2)
ITEM=$(grep -o 'ITEM=[^ ]*' "$LASTLOG" 2>/dev/null | tail -1 | cut -d= -f2)
HEAD=$(git log --oneline -1 2>/dev/null | cut -c1-60)
NEXT=$(python3 scripts/autonomy/state.py pick 2>/dev/null | cut -f1,4)

case "$OUT" in
  done)          say_once "done:$ITEM:$HEAD" "pii autonomy · $ITEM done" "$HEAD" ;;
  verify-failed|gate-failed|worker-rc-*|dirty-tree|bad-plan)
                 say_once "bad:$ITEM:$OUT" "pii autonomy · $ITEM needs a hand ($OUT)" "log: ${LASTLOG##*/}" ;;
  window-closed) say_once "closed:$(cut -c1-20 <<<"$(grep -o 'resets [^ ]* [^ ]*' "$LASTLOG" 2>/dev/null | tail -1)")" \
                   "pii autonomy · the Anthropic window is closed — nothing spent, the queue is untouched" ;;
esac

# 2b) an open question must reach the phone (the user's instruction: "qualquer duvida, pelo discord")
#     keyed by the question list itself, so the same question is not re-sent every tick
Q="${PII_QUESTIONS:-$ST/questions.md}"
if [ -f "$Q" ] && grep -q '^- \[ \]' "$Q"; then
  KEY="q:$(grep '^- \[ \]' "$Q" | md5sum | cut -c1-12)"
  OPEN=$(grep -c '^- \[ \]' "$Q")
  say_once "$KEY" "pii autonomy · $OPEN open question(s) for you:" "$(grep '^- \[ \]' "$Q" | head -3 | sed 's/^- \[ \] //' | cut -c1-160)"
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
