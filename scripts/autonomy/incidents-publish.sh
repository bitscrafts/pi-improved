#!/usr/bin/env bash
# incidents-publish.sh — the incident notification, in the repository, and LOUD when its record is missing.
#
# WHY THIS EXISTS. The publisher lived outside the repo (`~/.hermes/scripts/pii-incidents-discord.sh`) and its
# ninth line was `[ -f "$MD" ] || exit 0`. So "the detector never wrote a record" and "no incident happened"
# produced the same silence — a check that passes while the live path is broken, named as such by the 2026-11-02
# review. Measured that night: `_tmp/autonomy/INCIDENTS.md` did not exist at all.
#
# Behaviour:
#   · record missing      -> one line naming the path, exit 2 (absence is an ERROR, never an empty result)
#   · record but no rows  -> silence, exit 0 (a cron with empty stdout sends nothing)
#   · new rows            -> prints exactly those, once, and moves the cursor
#
# Env: PII_INCIDENTS_MD (default _tmp/autonomy/INCIDENTS.md), PII_INCIDENTS_SENT (default
# _tmp/autonomy/.incidents-sent), PII_PUBLISH_CMD (optional: a command the rows are piped to, e.g. a channel post).
set -uo pipefail
cd /projects/pi-improved || exit 1
MD=${PII_INCIDENTS_MD:-_tmp/autonomy/INCIDENTS.md}
CUR=${PII_INCIDENTS_SENT:-_tmp/autonomy/.incidents-sent}
PUB=${PII_PUBLISH_CMD:-}

# A missing record is an ERROR, but it is not news every quarter hour: a `no_agent` job delivers its stdout on every
# firing, so an unthrottled complaint becomes the noise it was written to replace. Loud once, then quiet until the
# window turns — still exit 2, so the job's status stays visibly wrong.
if [ ! -f "$MD" ]; then
  STAMP="${CUR}.missing"
  QUIET_H=${PII_PUBLISH_QUIET_H:-24}
  NOW=$(date -u +%s)
  LAST=$(cat "$STAMP" 2>/dev/null || echo 0)
  case "$LAST" in ''|*[!0-9]*) LAST=0 ;; esac
  if [ "$QUIET_H" -eq 0 ] || [ $((NOW - LAST)) -ge $((QUIET_H * 3600)) ]; then
    echo "incidents-publish: the incident record is missing at $MD — the detector has never written one"
    mkdir -p "$(dirname "$STAMP")" && printf '%s\n' "$NOW" >"$STAMP"
  fi
  exit 2
fi

mkdir -p "$(dirname "$CUR")"
SENT=$(cat "$CUR" 2>/dev/null || echo 0)
case "$SENT" in ''|*[!0-9]*) SENT=0 ;; esac
TOTAL=$(grep -cE '^[0-9]{4}-[0-9]{2}-[0-9]{2}T' "$MD" 2>/dev/null || true)
TOTAL=${TOTAL:-0}

# A record that shrank (rewritten, or replaced) means the cursor no longer describes it: start over rather than
# skip rows that were never printed.
[ "$TOTAL" -ge "$SENT" ] || SENT=0
[ "$TOTAL" -gt "$SENT" ] || exit 0

{
  echo "pii incident(s) opened:"
  grep -E '^[0-9]{4}-[0-9]{2}-[0-9]{2}T' "$MD" | tail -n $((TOTAL - SENT)) | sed 's/^/- /'
} | { if [ -n "$PUB" ]; then eval "$PUB"; else cat; fi; }

printf '%s\n' "$TOTAL" >"$CUR"
