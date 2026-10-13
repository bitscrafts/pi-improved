#!/usr/bin/env bash
# resume-proactivity-review.sh — the review that the account window cut off at turn 30. Reads the reset the CLI
# itself stated, probes cheaply once, and only then continues the SAME session. Prints four lines for the phone.
set -uo pipefail
cd /projects/pi-improved
. scripts/autonomy/env.sh
BR=/projects/_tmp/pii-review-20261013/brief-proactivity-self-improvement.md
OUT=/projects/_tmp/pii-review-20261013
SID=ef17e6d6-1741-42d6-87c6-16ffc775d231

if bash scripts/autonomy/window.sh check 2>/dev/null; then
  echo "pii review · still inside the closed window ($(bash scripts/autonomy/window.sh show)) — nothing spent"
  exit 0
fi
# A window probe must not demand an exact echo: measured 2026-10-13 16:55, the model answered "PONG" where the
# script asked for "PONG66" and the run was refused as a false negative while the account was open. An answer is an
# answer; only the CLI's refusal words mean closed.
PROBE=$(timeout 120 claude -p "Reply with a single word: PONG" --model sonnet --max-turns 1 2>&1); PRC=$?
if [ $PRC -ne 0 ] || [ -z "$(printf '%s' "$PROBE" | tr -d '[:space:]')" ] || printf '%s' "$PROBE" | grep -qiE 'hit your (session|weekly|monthly) limit|api_error_status|"is_error":true'; then
  U=$(bash scripts/autonomy/window.sh parse <(printf '%s' "$PROBE") 2>/dev/null || true)
  [ -n "$U" ] && echo "$U" > _tmp/autonomy/window-closed-until
  echo "pii review · the window is NOT open (rc=$PRC) — not dispatching; the loop stays parked"
  printf '%s\n' "$PROBE" | tail -2
  exit 0
fi
# the review and a worker share ONE account window: wait for the loop's lock rather than racing it
for i in $(seq 1 40); do
  if flock -n _tmp/autonomy/lock true 2>/dev/null; then break; fi
  [ $i -eq 1 ] && echo "pii review · a worker is in flight; waiting for the loop to free the window"
  sleep 30
done
echo "pii review · window open, resuming the review session (this takes ~20-30 min)"
TASK_LABEL=opus-proactivity-review REVIEW_MODE=test MAX_TURNS=140 \
  bash /root/.hermes/skills/research/opus-review/scripts/run-opus-review.sh /projects/pi-improved "$BR" "$OUT" --resume "$SID" >"$OUT/resume-attempt.log" 2>&1
RC=$?
echo "pii review · run rc=$RC"
grep -E 'exit=|session_id|stop_reason|cost_usd|result text|LIMIT|tree ' "$OUT/resume-attempt.log" | tail -8
exit 0
