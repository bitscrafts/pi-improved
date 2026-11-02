#!/usr/bin/env bash
# log-rotate.sh — bound the autonomy loop's own logs, which nothing was pruning.
#
# MEASURED 2026-11-02: _tmp/autonomy/logs held 49 MB in 7015 files, one cycle-*.log per cycle since the loop
# started, plus one tick-*.log per day. pii's daemon log already rotates by size (--log-rotate-bytes, keep 5) and
# Hermes rotates its own (agent.log.1/.2/.3), so this is specifically the loop's directory.
#
# THE RULE: a file is kept when it is younger than KEEP_DAYS; when the survivors still exceed MAX_FILES, the
# oldest of them go too. Every removal is appended to rotated.log, so the pruning is auditable after the fact.
#
# Usage: log-rotate.sh [--dry-run] [--apply]   (default: --dry-run)
#   LOGD=<dir>        directory to bound (default _tmp/autonomy/logs)
#   KEEP_DAYS=<n>     keep files modified within n days (default 7)
#   MAX_FILES=<n>     hard cap on how many files survive (default 400)
set -uo pipefail

APPLY=0
case "${1:-}" in
  --apply) APPLY=1 ;;
  --dry-run|"") APPLY=0 ;;
  *) echo "usage: $0 [--dry-run|--apply]" >&2; exit 2 ;;
esac

DEFAULT_LOGD=$(cd "$(dirname "$0")/../.." 2>/dev/null && pwd)/_tmp/autonomy/logs
# A guard must fail SAFE: a set-but-empty directory is a bug in the caller, not "use the default".
if [ "${LOGD+set}" = set ] && [ -z "${LOGD}" ]; then
  echo "log-rotate: REFUSED — LOGD is set but empty" >&2
  exit 2
fi
LOGD=${LOGD:-$DEFAULT_LOGD}
KEEP_DAYS=${KEEP_DAYS:-7}
MAX_FILES=${MAX_FILES:-400}
[ "${KEEP_DAYS+set}" = set ] && [ -z "${KEEP_DAYS}" ] && { echo "log-rotate: REFUSED — KEEP_DAYS empty" >&2; exit 2; }
[ "${MAX_FILES+set}" = set ] && [ -z "${MAX_FILES}" ] && { echo "log-rotate: REFUSED — MAX_FILES empty" >&2; exit 2; }

[ -d "$LOGD" ] || { echo "log-rotate: no such directory: $LOGD" >&2; exit 3; }
# Planning may look anywhere; deleting requires the directory to be a real log directory.
if [ "$APPLY" -eq 1 ]; then
  case "$LOGD" in
    *_tmp/autonomy/logs) : ;;
    *) if [ "${LOG_ROTATE_TEST:-0}" = "1" ]; then :; else
         echo "log-rotate: REFUSED — refusing to delete in $LOGD (not a log directory)" >&2; exit 4
       fi ;;
  esac
fi

LOG="$LOGD/rotated.log"
now=$(date +%s)
cutoff=$(( now - KEEP_DAYS * 86400 ))

# Old by age, then the oldest of what remains until the cap holds. rotated.log is never a candidate.
mapfile -t all < <(find "$LOGD" -maxdepth 1 -type f -name '*.log' ! -name 'rotated.log' -printf '%T@ %p\n' 2>/dev/null | sort -n | awk '{print $2}')
removed=0
kept=0
declare -A doomed=()

for f in "${all[@]}"; do
  m=$(stat -c %Y "$f" 2>/dev/null || echo "$now")
  if [ "$m" -lt "$cutoff" ]; then doomed["$f"]=1; fi
done

survivors=$(( ${#all[@]} - ${#doomed[@]} ))
if [ "$survivors" -gt "$MAX_FILES" ]; then
  excess=$(( survivors - MAX_FILES ))
  for f in "${all[@]}"; do
    [ "$excess" -le 0 ] && break
    [ -n "${doomed[$f]:-}" ] && continue
    doomed["$f"]=1; excess=$(( excess - 1 ))
  done
fi

for f in "${all[@]}"; do
  if [ -n "${doomed[$f]:-}" ]; then
    if [ "$APPLY" -eq 1 ]; then
      rm -f -- "$f" && printf '%s removed %s (%s bytes)\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$f" "$(stat -c %s "$f" 2>/dev/null || echo 0)" >> "$LOG"
    fi
    removed=$(( removed + 1 ))
  else
    kept=$(( kept + 1 ))
  fi
done

echo "log-rotate: $LOGD — $( [ "$APPLY" -eq 1 ] && echo removed || echo would remove) $removed, kept $kept (keep_days=$KEEP_DAYS max_files=$MAX_FILES)"
