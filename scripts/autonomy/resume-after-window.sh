#!/usr/bin/env bash
# resume-after-window.sh — when the Claude session window reopens, prove it with ONE turn, then drain the queue on haiku.
#
# Measured 2026-10-19 17:50Z: "You've hit your session limit · resets 9:10pm (UTC)". The cycle already refuses to retry
# before the printed reset (OUTCOME=window-closed), so nothing is wasted waiting — this script is what resumes the work
# after it, and it refuses to spend a single long turn until a 1-turn probe answers.
#
# Scheduled as a one-shot cron at reset + 5 minutes. Prints a short report; empty output would mean the probe failed
# silently, so every branch prints something.
set -uo pipefail
cd /projects/pi-improved || exit 1
export PATH="/root/.local/bin:/usr/local/cargo/bin:/usr/local/bin:/usr/bin:/bin:${PATH:-}"
export IS_SANDBOX="${IS_SANDBOX:-1}"

echo "resume-after-window: $(date -u +%H:%M:%SZ) — probing the window with one turn"

PROBE=$(timeout 90 claude -p "Reply with exactly: PONG55" --model claude-haiku-5-5 --max-turns 1 --output-format json 2>&1 | tail -c 400)
case "$PROBE" in
  *PONG55*) echo "probe OK — the window is open" ;;
  *'session limit'*|*'spend limit'*|*'resets'*)
      echo "probe REFUSED — still limited: $(printf '%s' "$PROBE" | grep -oiE 'resets[^"]{0,40}' | head -1)"
      echo "nothing was spent beyond the probe; the queue stays parked for the next window"
      exit 0 ;;
  *) echo "probe UNCLEAR (no PONG55 and no limit text) — not draining: $(printf '%s' "$PROBE" | tr '\n' ' ' | cut -c1-200)"
     exit 0 ;;
esac

echo "control: $(bash scripts/autonomy/control.sh status 2>/dev/null | head -1)"
export PII_WORKER=claude CLAUDE_MODEL=claude-haiku-5-5
for i in $(seq 1 6); do
  P=$(python3 scripts/autonomy/state.py pick 2>/dev/null | head -1)
  case "$P" in
    plan-error*) echo "STOP: $P"; break ;;
    "")          echo "QUEUE EMPTY"; break ;;
  esac
  echo "=== $i: $(printf '%s' "$P" | cut -f1) ==="
  bash scripts/autonomy/cycle.sh 2>&1 | tail -2
  sleep 5
done
python3 -c "
import json
s=json.load(open('_tmp/autonomy/state.json'))
open_items=[(k,(v or {}).get('status')) for k,v in sorted(s.items()) if isinstance(v,dict) and (v or {}).get('status') not in ('done','skipped')]
print('open:', open_items or 'none')
"
