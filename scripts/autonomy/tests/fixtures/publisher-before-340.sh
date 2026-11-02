#!/usr/bin/env bash
# pii-incidents-discord.sh — the Discord surface for the loop's incidents (cron: no_agent, deliver=all).
#
# The file surface (_tmp/autonomy/INCIDENTS.md, published by scripts/autonomy/incidents-md.sh) is the record; this is
# the notification. It prints ONLY rows it has not printed before, and prints NOTHING when there are none -- with
# no_agent a silent stdout sends no message, which is what keeps a 15-minute job from becoming noise.
set -uo pipefail
MD=${PII_INCIDENTS_MD:-/projects/pi-improved/_tmp/autonomy/INCIDENTS.md}
CUR=${PII_INCIDENTS_SENT:-/root/.hermes/state/pii-incidents-sent}
[ -f "$MD" ] || exit 0
mkdir -p "$(dirname "$CUR")"
SENT=$(cat "$CUR" 2>/dev/null || echo 0)
case "$SENT" in ''|*[!0-9]*) SENT=0 ;; esac
TOTAL=$(grep -cE '^[0-9]{4}-[0-9]{2}-[0-9]{2}T' "$MD" 2>/dev/null; true)
[ "$TOTAL" -gt "$SENT" ] || exit 0
{
  echo "pii incident(s) opened:"
  grep -E '^[0-9]{4}-[0-9]{2}-[0-9]{2}T' "$MD" | tail -n $((TOTAL - SENT)) | sed 's/^/- /'
} 
printf '%s\n' "$TOTAL" > "$CUR"
