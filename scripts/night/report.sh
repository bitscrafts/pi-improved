#!/usr/bin/env bash
# report.sh — the morning report: how many Opus analyses ran, what each one asked for, and what the drain closed
# after it. Every line is read from a file or a command; nothing here is recalled.
#
# Usage: report.sh [--since <ISO8601>]     (default: 2026-11-02T05:20:00Z — the first audit's start)
# Output: plain text, compact, no pipe tables (it is read on a phone in Discord).
set -uo pipefail
cd /projects/pi-improved || exit 1
SINCE=${2:-2026-11-02T05:20:00Z}
NOW=$(date -u +%Y-%m-%dT%H:%M:%SZ)

# 1. every Opus review that produced a document: the audit, then one per night review
mapfile -t REVIEWS < <(ls -1 _docs/06-research/opus-status-and-path-*.md _docs/06-research/night-review-*.md 2>/dev/null | sort)
echo "pii — the night of $(date -u -d "$SINCE" '+%d %b'), report at $NOW"
echo "Opus analyses: ${#REVIEWS[@]}"
echo

# the cost and turns of each run live beside the report, in the runner's json
for f in "${REVIEWS[@]}"; do
  base=$(basename "$f" .md)
  when=$(date -u -r "$f" +%H:%M)
  cost="?"; turns="?"
  for j in _tmp/opus-status-20261102/opus-review-*.json _tmp/night/opus-*/*.json; do
    [ -f "$j" ] || continue
    c=$(python3 -c "import json,sys;d=json.load(open('$j'));print(d.get('total_cost_usd') or d.get('cost') or '')" 2>/dev/null)
    t=$(python3 -c "import json,sys;d=json.load(open('$j'));print(d.get('num_turns') or '')" 2>/dev/null)
    [ -n "${c:-}" ] && cost=$c && turns=${t:-?}
  done
  echo "== $(printf '%s' "$base" | sed 's/^opus-status-and-path-/audit /; s/^night-review-/review /')  ($when)"
  echo "   file   : $f"
  # the review's own single next action, verbatim from its last section
  na=$(awk '/^## The single next action/{flag=1;next} flag&&NF&&!/^#/{print;exit}' "$f")
  [ -n "$na" ] && echo "   asked  : $(printf '%s' "$na" | cut -c1-220)"
  echo
done

# 2. what the drain closed, in order, with times
echo "Committed between $SINCE and $NOW:"
git log --since="$SINCE" --format='   %ad  %h  %s' --date=format:'%H:%M' | cut -c1-150
echo

# 3. the queue rows this night touched
echo "Rows 335-340:"
python3 scripts/autonomy/state.py list 2>/dev/null | awk -F'\t' '$1>=335 && $1<=340 {printf "   %s  %-9s attempts=%s  %s\n", $1, $4, $5, substr($3,1,90)}' 
echo

# 4. the loop's own journal, last lines — what it decided and when
echo "The night's journal (last 8 lines):"
tail -8 _tmp/night/night.log 2>/dev/null | sed 's/^/   /'
echo

# 5. anything parked: the honest part
parked=$(python3 scripts/autonomy/state.py list 2>/dev/null | awk -F'\t' '$4=="parked" || $4=="needs-review"' | wc -l | tr -d ' ')
gates=$(bash scripts/checks/gates.sh 2>&1 | tail -1)
echo "Parked or needs-review: $parked row(s)"
echo "Gates: $gates"
echo "Tree: $(git status --porcelain | wc -l | tr -d ' ') changed path(s) at HEAD $(git log --format='%h' -1)"
