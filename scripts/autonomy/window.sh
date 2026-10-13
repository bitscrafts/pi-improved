#!/usr/bin/env bash
# window.sh — the account's window, read from the CLI's own words. The rule this implements is the project's
# oldest operational one: **read the reset time the CLI gives, never blind-retry, never shrink scope**
# (AGENTS.md, quota policy; DO-NOT-REPEAT 2.8). Measured 2026-10-13: a worker was refused with
# `"api_error_status":429, "You've hit your session limit · resets 4:50pm (UTC)"` while the 1-turn probe still
# said OPEN — so the probe alone is not the guard; the worker's own log is.
set -uo pipefail
STATEDIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../_tmp/autonomy" && pwd)"
MARK="$STATEDIR/window-closed-until"

usage() { echo "usage: window.sh parse <logfile> | check | clear | show" >&2; exit 2; }

# parse: from a worker log, print the epoch when the window reopens, or nothing if the log shows no refusal
case "${1:-}" in
parse)
  LOG="${2:-}"; [ -n "$LOG" ] && [ -f "$LOG" ] || { echo "window.sh: no such log: ${2:-}" >&2; exit 2; }
  SIG=$(grep -oE 'hit your (session|weekly|monthly) limit|api_error_status":"?429|resets [0-9]{1,2}:[0-9]{2}(am|pm) \(UTC\)' "$LOG" | head -1)
  [ -n "$SIG" ] || exit 0                      # no refusal in this log: print nothing
  R=$(grep -oE 'resets [0-9]{1,2}:[0-9]{2}(am|pm) \(UTC\)' "$LOG" | tail -1)
  python3 - "$R" <<'PY'
import datetime, re, sys
now = datetime.datetime.now(datetime.timezone.utc)
raw = sys.argv[1] if len(sys.argv) > 1 else ""
m = re.search(r'resets (\d{1,2}):(\d{2})(am|pm)', raw)
if not m:
    # the log says we are refused but not when: assume the shortest sane wait, and say so by the value's size
    print(int((now + datetime.timedelta(minutes=10)).timestamp())); raise SystemExit(0)
h, mi, ap = int(m.group(1)) % 12, int(m.group(2)), m.group(3)
h += 12 if ap == "pm" else 0
when = now.replace(hour=h, minute=mi, second=0, microsecond=0)
if when <= now:                       # the stated time already passed: it means the next window, not this one
    when += datetime.timedelta(hours=5)
print(int(when.timestamp()))
PY
  ;;
check)
  [ -f "$MARK" ] || exit 1
  UNTIL=$(cat "$MARK" 2>/dev/null || echo 0)
  [ -n "$UNTIL" ] && [ "$(date -u +%s)" -lt "$UNTIL" ] && exit 0
  exit 1
  ;;
clear) rm -f "$MARK"; echo "window: open (marker cleared)"; ;;
show)
  if [ -f "$MARK" ]; then echo "closed until $(date -u -d "@$(cat "$MARK")" '+%Y-%m-%dT%H:%M:%SZ')"; else echo "open"; fi
  ;;
*) usage ;;
esac
