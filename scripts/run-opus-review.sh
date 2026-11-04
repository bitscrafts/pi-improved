#!/usr/bin/env bash
# run-opus-review.sh <brief.md> <slug> — one READ-ONLY Opus review of a brief, on a named session.
#
# House rule: the brief is the operator's/orchestrator's, never the reviewer's; the reviewer may not
# write to the tree. Everything the run needs is set INSIDE this script, because a background shell
# inherits nothing (the failures of 2026-10-01 all began with an inline dispatch that worked only in
# the shell that launched it).
#
#   bash scripts/run-opus-review.sh _docs/06-research/045-vocabulary-review-brief.md vocab-review
#   DRY_RUN=1 bash scripts/run-opus-review.sh ...        # print the plan, spend nothing
#
# Exit: 0 ran or dry-ran · 2 brief missing/empty · 1 the run failed. Never commits, never edits.
set -uo pipefail

REPO=/projects/pi-improved
BRIEF=${1:?usage: run-opus-review.sh <brief.md> <slug>}
SLUG=${2:?usage: run-opus-review.sh <brief.md> <slug>}
CLAUDE=${CLAUDE_BIN:-claude}
MODEL=${CLAUDE_MODEL:-claude-opus-5-5}
WORK=/projects/_tmp/claude-batchC
LOG="$WORK/$SLUG.$(date +%s).log"
SIDFILE="$WORK/session-$SLUG.txt"

export IS_SANDBOX="${IS_SANDBOX:-1}"          # root + bypassPermissions requires it (2026-09-28)
export PATH="/root/.local/bin:/usr/local/bin:/usr/bin:/bin:${PATH:-}"
cd "$REPO" || exit 1

[ -s "$BRIEF" ] || { echo "ABORT: brief missing or empty: $BRIEF" >&2; exit 2; }
mkdir -p "$WORK"

echo "BRIEF   : $BRIEF ($(wc -c <"$BRIEF") bytes)"
echo "MODEL   : $MODEL   (read-only: --permission-mode plan)"
echo "LOG     : $LOG"
echo "SESSION : will be recorded to $SIDFILE"
echo "TREE    : $(git status --porcelain | wc -l) path(s) dirty before the run (the review must not add any)"

if [ "${DRY_RUN:-0}" = "1" ]; then
  echo "DRY RUN — nothing spent."
  exit 0
fi

BEFORE=$(git status --porcelain | wc -l)
"$CLAUDE" --print \
  --permission-mode plan \
  --model "$MODEL" \
  --session-id "$(uuidgen 2>/dev/null || cat /proc/sys/kernel/random/uuid)" \
  -p "$(cat "$BRIEF")" >"$LOG" 2>&1
RC=$?
AFTER=$(git status --porcelain | wc -l)

SID=$(grep -oE '"session_id":"[^"]+"' "$LOG" 2>/dev/null | head -1 | cut -d'"' -f4)
[ -n "$SID" ] && printf '%s\n' "$SID" >"$SIDFILE" && echo "SESSION : $SID (resume: --resume $SID)"

echo "rc=$RC  log=$LOG"
echo "tree: before=$BEFORE after=$AFTER  (a review that changed the tree is a failed review)"
tail -3 "$LOG"
[ "$RC" -eq 0 ] && [ "$BEFORE" = "$AFTER" ]
