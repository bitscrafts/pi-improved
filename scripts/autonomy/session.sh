#!/usr/bin/env bash
# session.sh recover <spec-number> | check <session-id>
# A worker cut off mid-flight (the account window, a kill) leaves its session id in its own stream log even when the
# dispatcher never reached the line that writes session-<N>-*.txt. That id is the resume handle, and losing it costs
# a whole re-implementation of the same spec (measured 2026-10-13, spec 153).
set -uo pipefail
case "${1:-}" in
recover)
  N="${2:-}"; [ -n "$N" ] || { echo "usage: session.sh recover <spec-number>" >&2; exit 2; }
  # newest first: the dispatcher's own record wins, then the worker's stream log
  S=$(cat /projects/pi-improved/_tmp/claude-batchC/session-"$N"-*.txt 2>/dev/null | awk '{print $1}' | tail -1)
  if [ -z "$S" ]; then
    L=$(ls -t /projects/pi-improved/_tmp/claude-batchC/"$N"-*.log 2>/dev/null | head -1)
    [ -n "$L" ] && S=$(grep -oE '"session_id":"[0-9a-f-]{36}"' "$L" 2>/dev/null | tail -1 | cut -d'"' -f4)
  fi
  printf '%s' "$S"
  ;;
check)
  S="${2:-}"; [ -n "$S" ] || exit 1
  ls /root/.claude/projects/*/"$S".jsonl >/dev/null 2>&1 && exit 0 || exit 1
  ;;
*) echo "usage: session.sh recover <spec-number> | check <session-id>" >&2; exit 2 ;;
esac
