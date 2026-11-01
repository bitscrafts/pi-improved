#!/usr/bin/env bash
# incidents-md.sh — the incident surface that exists today: a file.
#
# Why a file (user, 2026-11-01: "lets us a file by now!"): the loop already OPENS incidents (spec 150) and the tick
# already FORMATS one line per new incident, but the line had two homes and neither was readable — `tick.sh`'s
# output went to /dev/null for eleven days (fixed 2026-11-01), and the delivering cron job has been disabled since
# 2026-10-19. So the record existed and no reader could see it. A file has no credential, no daemon, no restart and
# no delivery contract: it is the honest interim surface while the Discord path is decided.
#
# Append-only. Duplicate-free: a cursor counts the incident lines already published, so re-running never repeats one.
# usage: incidents-md.sh [--check]
set -uo pipefail
cd "$(dirname "$(readlink -f "$0")")/../.." || exit 1
LOGD=${AUTONOMY_LOGD:-_tmp/autonomy/logs}
MD=${PII_INCIDENTS_MD:-_tmp/autonomy/INCIDENTS.md}
CUR=${PII_INCIDENTS_CURSOR:-_tmp/autonomy/INCIDENTS.cursor}
CHECK=0
[ "${1:-}" = "--check" ] && CHECK=1

# every incident line the tick has ever printed, oldest first
all_lines(){
  local f
  for f in $(ls -1 "$LOGD"/tick-*.log 2>/dev/null | sort); do
    grep -h 'INCIDENT ' "$f" 2>/dev/null
  done
}

TOTAL=$(all_lines | wc -l)
SEEN=$(cat "$CUR" 2>/dev/null || echo 0)
case "$SEEN" in ''|*[!0-9]*) SEEN=0 ;; esac

if [ "$CHECK" = 1 ]; then
  if [ "$TOTAL" -gt "$SEEN" ]; then
    echo "incidents: $((TOTAL - SEEN)) unpublished incident line(s) ($TOTAL seen, $SEEN published to $MD)"
    exit 3
  fi
  echo "incidents: up to date ($SEEN line(s) published to $MD)"
  exit 0
fi

if [ "$TOTAL" -le "$SEEN" ]; then
  echo "incidents: up to date ($SEEN line(s) published to $MD)"
  exit 0
fi

if [ ! -f "$MD" ]; then
  {
    echo "# INCIDENTS — one line per incident the loop opened, newest last"
    echo
    echo "Written by scripts/autonomy/incidents-md.sh from the autonomy log. Append-only; deleting a line here does"
    echo "not close an incident, because the source of truth is the incidents record the scan writes."
    echo
  } > "$MD"
fi

N=0
while IFS= read -r line; do
  N=$((N + 1))
  [ "$N" -le "$SEEN" ] && continue
  # "pii autonomy · INCIDENT <fingerprint> — <count> times since <iso> (see <path>)" -> a dated, greppable row
  FP=$(printf '%s' "$line" | sed -n 's/.*INCIDENT \([^ ]*\) .*/\1/p')
  REST=$(printf '%s' "$line" | sed 's/^.*INCIDENT [^ ]* //')
  printf '%s | %s | %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$FP" "$REST" >> "$MD"
done <<EOF
$(all_lines)
EOF

printf '%s\n' "$TOTAL" > "$CUR"
echo "incidents: published $((TOTAL - SEEN)) new line(s) to $MD ($TOTAL total)"
