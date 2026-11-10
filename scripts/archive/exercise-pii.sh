#!/usr/bin/env bash
# exercise-pii.sh (v2) — INJECT faults, then ONLY OBSERVE. Nothing in this script heals anything.
#
# The rule, from the user 2026-10-18: "you just observe, the system should heal".
# So every fault is injected and LEFT IN PLACE while we watch whether pii recovers by itself:
#     systemd restarts the daemon · a heal node runs · the loop acts · the reader opens an incident
# If a fault is still in place at the end, that is a FINDING, not a cleanup step. Reversing it is a separate,
# deliberate operator action — never something a test does to make its own result look good.
#
# Run:  bash scripts/archive/exercise-pii.sh
set -uo pipefail
cd /projects/pi-improved || exit 1
export XDG_RUNTIME_DIR=/run/user/0
export PII_RUNTIME_DIR=/run/user/0/pii
HOST=mvcorrea@10.10.10.217
PII=/root/.local/bin/pii
LOG=/root/.local/state/pii/pii.log
CONF=/root/.config/pii/pii.toml

up()    { ssh -o BatchMode=yes "$HOST" 'systemctl --user is-active pii' 2>&1 | tail -1; }
kinds() { grep -ao '"kind":"[a-z._]*"' "$LOG" 2>/dev/null | sort | uniq -c | sort -rn | head -8; }
inc()   { "$PII" incidents scan --source runtime --log "$LOG" 2>&1 | tail -3 | cut -c1-155; }
cronn() { "$PII" health --json 2>/dev/null | python3 -c 'import json,sys; print((json.load(sys.stdin).get("counts") or {}).get("cron_entries"))' 2>/dev/null || echo "?"; }

echo "############ F1 — kill -9. OBSERVE ONLY: does pii return by itself?"
ssh -o BatchMode=yes "$HOST" 'pkill -9 -f "pii daemon" || true' >/dev/null 2>&1
for i in 1 2 3 4 5 6; do
  sleep 10
  echo -n "  t+$((i*10))s unit="; up
done
echo "  incidents the reader derives:"; inc

echo
echo "############ F2 — a [[cron]] whose graph does not exist. OBSERVE ONLY: is it refused, does anything retry it, does it clear?"
cp "$CONF" "${CONF}.pre-exercise"
printf '\n[[cron]]\nname = "broken"\ngraph = "no-such-graph"\nschedule = "0 12 * * *"\n' >> "$CONF"
ssh -o BatchMode=yes "$HOST" 'systemctl --user restart pii && sleep 8 && systemctl --user is-active pii' 2>&1 | tail -1
echo "  refusal events:"
grep -a 'cron.start.refused' "$LOG" 2>/dev/null | tail -2 | cut -c1-155
for i in 1 2 3; do
  sleep 10
  echo -n "  t+$((i*10))s cron_entries="; cronn
done
echo "  NOTE: the broken config is LEFT IN PLACE. Healing it is not this script's job — if pii cannot survive a bad"
echo "        config entry and say so plainly, that is the finding."

echo
echo "############ SUMMARY — what pii NOTICED, and whether it recovered ON ITS OWN"
echo "  event kinds across the log:"
kinds
echo
echo "  incidents:"
inc
echo
echo "  config: a broken cron is still in $CONF; the pre-exercise copy is ${CONF}.pre-exercise"
echo "  unit:   $(up)"
