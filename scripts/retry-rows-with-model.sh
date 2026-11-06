#!/usr/bin/env bash
# retry-rows-with-model.sh <model> <row>... — retry rows with ONE named model, through the normal cycle.
#
# Why it exists (2026-11-05, operator: "changed the model. retry the failed! dispatch with claude sonnet 5.5"):
# the dispatcher's default worker is claude-haiku-5-5 (drain.sh:77). Re-running a failed row with a different
# model must not be an inline command — it is the most expensive action the project takes — so it is this
# script: reviewable before it runs, identical when re-run, model and rows printed before anything is spent.
#
# What it does, per row, in the order given:
#   1. print the row's state, then reset it to pending with attempts 0 and a note saying WHY (a retry with a
#      different model is not a continuation of the old attempts, and the note keeps the history honest);
#   2. run scripts/autonomy/cycle.sh with PII_FORCE_ITEM=<row> and CLAUDE_MODEL=<model>: the same worker,
#      criteria execution, gate and commit the dispatcher uses. Nothing here bypasses a guard.
#   3. stop at the first row whose cycle does not END in a commit-or-done outcome, leaving the tree as it is.
#
# The model is scoped to THIS PASS. CLAUDE_MODEL is set on the cycle's own environment and nowhere else: the
# dispatcher's default (claude-haiku-5-5, drain.sh:77) is never edited, so the moment this script exits the
# queue is back on haiku. To keep the cron from dispatching a haiku row BETWEEN two of this pass's rows, the
# script holds the tree for its whole run and releases it on every exit path (trap), so the hold cannot
# outlive the pass. The cron stays ON throughout ("cron must always be on"); it only yields.
# It does NOT commit by itself: cycle.sh runs the worker, the criteria, the gate and the commit.
set -uo pipefail
cd /projects/pi-improved || exit 1
export PATH=/root/.local/bin:/usr/local/cargo/bin:/usr/local/bin:/usr/bin:/bin:${PATH:-}
MODEL="${1:-}"; shift || true
[ -n "$MODEL" ] && [ $# -ge 1 ] || { echo "usage: retry-rows-with-model.sh <model> <row>..."; exit 2; }
case "$MODEL" in claude-*) ;; *) echo "refusing: '$MODEL' is not a claude model id (this lane runs the claude CLI)"; exit 2 ;; esac

# A pass may carry a per-run budget (PII_BUDGET_USD): the reviewer model spent the whole 1.00 default reading and writing
# tests on row 392 (measured 2026-11-06). It is exported to cycle.sh as PII_MAX_BUDGET_USD on THIS pass's environment
# only, exactly as CLAUDE_MODEL is; with none given nothing is set and the queue's own default decides.
BUDGET="${PII_BUDGET_USD:-}"
if [ -n "$BUDGET" ]; then
  awk -v b="$BUDGET" 'BEGIN { exit !(b ~ /^[0-9]+(\.[0-9]+)?$/ && b + 0 > 0) }' || { echo "refusing: PII_BUDGET_USD='$BUDGET' is not a positive number (a budget of this pass)"; exit 2; }
fi
echo "MODEL    : $MODEL   (provider: anthropic, the claude login inside the devbox)"
echo "ROWS     : $*"
echo "BUDGET   : ${BUDGET:-queue default} per run"
echo "STARTED  : $(date -u +%FT%TZ)   HEAD $(git rev-parse --short HEAD)"
# A dirty tree is the FIRST row's own unfinished work when every dirty path is one that row declares (a worker cut off
# or a cheaper model that got part-way leaves exactly that): the cycle resumes it, it does not discard it. Anything
# else dirty is somebody else's and stops the pass. unlisted-changes.sh is the same test the commit step applies, and
# exits 0 for a clean tree.
FIRST="$1"
FIRST_PATHS=$(awk -F'\t' -v r="$FIRST" '$1==r {print $6}' scripts/autonomy/plan.tsv)
bash scripts/autonomy/unlisted-changes.sh "$FIRST_PATHS" >/tmp/retry-unowned.$$ 2>&1 || { echo "STOPPED: the tree has changes row $FIRST does not own; commit or park them first"; head /tmp/retry-unowned.$$; rm -f /tmp/retry-unowned.$$; exit 8; }
rm -f /tmp/retry-unowned.$$
[ -z "$(git status --porcelain)" ] || echo "NOTE: the tree holds row $FIRST's own unfinished work ($(git status --porcelain | wc -l) paths); the cycle resumes it"

OWNER="retry-$MODEL-$$"
bash scripts/autonomy/hold.sh take "$OWNER" 14400 100 || { echo "STOPPED: could not take the tree hold"; exit 8; }
trap 'bash scripts/autonomy/hold.sh release "$OWNER" >/dev/null 2>&1; echo "RELEASED : hold $OWNER — the dispatcher is back on its default model (claude-haiku-5-5)"' EXIT

for row in "$@"; do
  echo; echo "════ row $row"
  python3 scripts/autonomy/state.py get "$row" >/dev/null 2>&1 || { echo "no such row: $row"; exit 2; }
  echo "   was: $(python3 scripts/autonomy/state.py get "$row" | tr -d '\n ' | cut -c1-140)"
  python3 scripts/autonomy/state.py set "$row" status=pending attempts=0 >/dev/null
  python3 scripts/autonomy/state.py repair "$row" "retried with $MODEL: the earlier attempts used another model and died at the coder" >/dev/null 2>&1
  python3 scripts/autonomy/state.py set "$row" status=pending attempts=0 >/dev/null
  OUT=$( [ -n "$BUDGET" ] && export PII_MAX_BUDGET_USD="$BUDGET"; PII_FORCE_ITEM="$row" CLAUDE_MODEL="$MODEL" bash scripts/autonomy/cycle.sh 2>&1 | tail -3)
  echo "$OUT" | sed 's/^/   /' | cut -c1-160
  LAST=$(printf '%s\n' "$OUT" | grep -o 'OUTCOME=[a-z0-9-]*' | tail -1)
  echo "   $LAST   HEAD $(git rev-parse --short HEAD)   tree dirty: $(git status --porcelain | wc -l)"
  case "$LAST" in
    # the outcomes cycle.sh prints when a row ended GREEN and was committed (or was already satisfied)
    OUTCOME=done|OUTCOME=done-satisfied|OUTCOME=item-already-closed) ;;
    *) echo "STOPPING at row $row: $LAST — the tree and the row are left as they are for inspection"; exit 1 ;;
  esac
done
echo; echo "FINISHED : $(date -u +%FT%TZ)   HEAD $(git rev-parse --short HEAD)"
echo "rows now : $(for r in "$@"; do printf '%s=%s ' "$r" "$(python3 scripts/autonomy/state.py get "$r" | python3 -c 'import sys,json;print(json.load(sys.stdin)["status"])')"; done)"
