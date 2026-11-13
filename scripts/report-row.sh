#!/usr/bin/env bash
# Report a landed row on the issue it came from, and close that issue when the row is done.
# Usage: scripts/report-row.sh <row> --issue <N> [owner/repo]
#
# Spec 419: the round trip was open at its last step — a row landed and the issue that asked for it was closed by hand.
# This is the orchestrator's command AFTER a landing, not a step inside it: the queue stays offline, and a landing can
# never be blocked by GitHub. Its guarantee is the one spec 417 learned: the claim is what the read-back says.
#
# Spec 421: every comment pii writes is two lines — the sentence a person reads, and a marker a script reads that renders
# invisible. "Already reported" is decided by the FIRST LINE of a comment, never by a substring found anywhere in the body:
# a person who quotes pii's comment in a reply must not be mistaken for pii.
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
INSTANCE="${PII_INSTANCE:-}"
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

# The visible sentence, then the machine marker (invisible once rendered).
TEXT="pii: row $ROW $STATUS at $COMMIT — $SPEC"
MARKER="<!-- pii kind=row row=$ROW commit=$COMMIT${INSTANCE:+ instance=$INSTANCE} -->"
BODY="$(printf '%s\n%s' "$TEXT" "$MARKER")"
# first_lines: the first line of every comment, so "already reported" is a first-line comparison.
first_lines() { gh issue view "$ISSUE" --repo "$REPO" --json comments -q '.comments[] | .body | split("\n")[0]'; }
state_of() { gh issue view "$ISSUE" --repo "$REPO" --json state -q .state; }

# R3/R6: at most one comment per (row, status, commit), decided on the first line only.
if first_lines | grep -qxF "$TEXT"; then
  echo "already reported: #$ISSUE <- $TEXT"
else
  gh issue comment "$ISSUE" --repo "$REPO" --body "$BODY" >/dev/null
  # R4: the claim is the read-back, never the call's exit status.
  if ! first_lines | grep -qxF "$TEXT"; then
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
