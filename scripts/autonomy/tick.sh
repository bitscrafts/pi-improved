#!/usr/bin/env bash
# tick.sh — one scheduled tick, no LLM involved.
#
# This is the engine's heartbeat: it either starts exactly one cycle, or reports the one that is running,
# and prints three lines (which the scheduler delivers verbatim to the phone). It deliberately runs no agent:
# a cron agent inherits the session's model/provider and dies on a 90 s API timeout before it can run anything
# (measured 2026-10-10 and again 2026-10-11 — the first tick of the autonomy job fell from gemini to an nvidia
# fallback and then timed out). The deterministic script cannot be starved by a provider.
#
# The judgement (authoring a spec, reddening a mutation by hand, judging a live proof) is a separate,
# best-effort supervisor job — if that one dies on a provider error, this loop keeps going.
set -uo pipefail
cd /projects/pi-improved || { echo "pii autonomy: cannot cd to the project"; exit 1; }
ST=_tmp/autonomy; LOGD=$ST/logs; mkdir -p "$LOGD"

# 1) is a cycle already running? (flock on the same file the cycle uses; touching nothing)
if ! flock -n "$ST/lock" true 2>/dev/null; then
  AGE=$(( $(date +%s) - $(stat -c %Y "$ST/lock" 2>/dev/null || date +%s) ))
  LAST=$(ls -t "$LOGD"/cycle-*.log 2>/dev/null | head -1)
  ITEM=$(python3 scripts/autonomy/state.py list 2>/dev/null | awk -F'\t' '$4!="done"{print $1" "$3; exit}')
  echo "pii autonomy · a cycle is in flight (${AGE}s)"
  echo "working on: ${ITEM:-unknown}"
  echo "log: ${LAST##*/}"
  exit 0
fi

# 2) the last cycle's outcome, for the record line
LASTLOG=$(ls -t "$LOGD"/cycle-*.log 2>/dev/null | head -1)
LASTLINE=$([ -n "$LASTLOG" ] && grep -o 'OUTCOME=[^ ]*' "$LASTLOG" | tail -1 || echo "OUTCOME=none-yet")

# 3) start exactly one cycle, detached, and say so
NEXT=$(python3 scripts/autonomy/state.py pick 2>/dev/null | cut -f1,4)
setsid nohup bash scripts/autonomy/cycle.sh >>"$LOGD/cycle-last-detached.log" 2>&1 </dev/null &
sleep 2
RUNNING=$(flock -n "$ST/lock" true 2>/dev/null && echo no || echo yes)
echo "pii autonomy · tick · previous ${LASTLINE#OUTCOME=}"
echo "started: ${NEXT:-queue empty}"
echo "cycle running: ${RUNNING} · log: ${LASTLOG##*/}"
