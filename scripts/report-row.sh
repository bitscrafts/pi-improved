#!/usr/bin/env bash
# Report a landed row on the issue it came from, and close that issue when the row is done.
# Usage: scripts/report-row.sh <row> --issue <N> [owner/repo]
#
# Spec 419: the round trip was open at its last step — a row landed and the issue that asked for it was closed by hand.
# This is the orchestrator's command AFTER a landing, not a step inside it: the queue stays offline, and a landing can
# never be blocked by GitHub. Its guarantee is the one spec 417 learned: the claim is what the read-back says.
#
# The comment is one fixed sentence; nothing from the issue's own text and nothing from a worker's output is echoed.
# A second run with the same row, status and commit posts nothing, so a re-run after a failed call is safe.
set -eu
ROW="${1:?usage: report-row.sh <row> --issue <N> [owner/repo]}"; shift
ISSUE=""; REPO=""
while [ $# -gt 0 ]; do
  case "$1" in
    --issue) ISSUE="${2:?--issue needs a number}"; shift 2 ;;
    -*) echo "REFUSE: unknown flag $1"; exit 2 ;;
    *) REPO="$1"; shift ;;
  esac
done
[ -n "$ISSUE" ] || { echo "REFUSE: --issue is required"; exit 2; }
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || exit 2
cd "$ROOT" || exit 2
STATE="${PII_STATE:-$ROOT/_tmp/autonomy/state.json}"
REPO="${REPO:-$(gh repo view --json nameWithOwner -q .nameWithOwner)}"

# R1: the row's own record, read once. An unknown row is refused before anything is posted.
INFO="$(python3 - "$STATE" "$ROW" <<'PY'
import json, sys
try:
    state = json.load(open(sys.argv[1]))
except (OSError, ValueError):
    sys.exit(3)
row = state.get(sys.argv[2])
if not isinstance(row, dict):
    sys.exit(3)
print(row.get("status") or "pending")
print(row.get("commit") or "-")
print(row.get("spec") or "-")
PY
)" || { echo "REFUSE: row $ROW has no state in $STATE"; exit 3; }
STATUS="$(printf '%s\n' "$INFO" | sed -n 1p)"
COMMIT="$(printf '%s\n' "$INFO" | sed -n 2p)"
SPEC="$(printf '%s\n' "$INFO" | sed -n 3p)"
TEXT="pii row $ROW $STATUS at $COMMIT: $SPEC"
comments() { gh issue view "$ISSUE" --repo "$REPO" --json comments -q '.comments[].body'; }
state_of() { gh issue view "$ISSUE" --repo "$REPO" --json state -q .state; }

# R3: at most one comment per (row, status, commit).
ALREADY="$(comments || true)"
if printf '%s\n' "$ALREADY" | grep -qxF "$TEXT"; then
  echo "already reported: #$ISSUE <- $TEXT"
else
  gh issue comment "$ISSUE" --repo "$REPO" --body "$TEXT" >/dev/null
  # R4: the claim is the read-back, never the call's exit status.
  if ! comments | grep -qxF "$TEXT"; then
    echo "FAIL: #$ISSUE does not carry the comment after posting"
    exit 5
  fi
  echo "commented: #$ISSUE <- $TEXT"
fi

# R2: nothing is closed unless the row is done; R4: closing is read back too.
if [ "$STATUS" = done ]; then
  if [ "$(state_of)" != CLOSED ]; then
    gh issue close "$ISSUE" --repo "$REPO" --reason completed >/dev/null
    if [ "$(state_of)" != CLOSED ]; then
      echo "FAIL: #$ISSUE is not CLOSED after closing it"
      exit 6
    fi
    echo "closed: #$ISSUE (completed)"
  else
    echo "already closed: #$ISSUE"
  fi
else
  echo "left open: #$ISSUE (row is $STATUS)"
fi
