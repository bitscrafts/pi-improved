#!/usr/bin/env bash
# morning-report.sh — the 07:00 local report. Its STDOUT becomes the Discord message, so it must be self-contained,
# compact and readable on a phone. No model: every line is read from a file or a command, which is the only way a
# report can be trusted at the hour nobody is awake to check it.
set -uo pipefail
cd /projects/pi-improved || exit 1

echo "pii — the night of 02 Nov (07:00 local)"
echo

# 1. how many Opus analyses ran, and what each one asked for
mapfile -t REVIEWS < <(ls -1 _docs/06-research/opus-status-and-path-*.md _docs/06-research/night-review-*.md 2>/dev/null | sort)
echo "OPUS ANALYSES: ${#REVIEWS[@]}"
i=0
for f in "${REVIEWS[@]}"; do
  i=$((i+1))
  when=$(date -u -r "$f" +%H:%M)
  asked=$(awk '/^## The single next action/{flag=1;next} flag&&NF&&!/^#/{print;exit}' "$f" | cut -c1-200)
  echo "$i. $when  $(basename "$f")"
  echo "   asked for: ${asked:-（no 'single next action' section — it was a status review）}"
done
echo

# 2. what the drain closed, with times and shas
echo "CLOSED (commits since the first analysis):"
git log --since="2026-11-02T05:20:00Z" --format='· %ad %h %s' --date=format:'%H:%M' | cut -c1-120
echo

# 3. rows and their state — the part that says what did NOT happen
echo "QUEUE:"
python3 scripts/autonomy/state.py list 2>/dev/null | awk -F'\t' '$1>=335 && $1<=340 {printf "· %s %-12s attempts=%s  %s\n", $1, $4, $5, substr($3,1,70)}'
echo

# 4. gates: reuse the last verdict if a worker is mid-flight (a gate run then measures the worker, not the tree)
if pgrep -f 'run-batchC-pii|scripts/autonomy/cycle\.sh' >/dev/null 2>&1; then
  last=$(grep -h 'VERDICT\|fresh-gate:' _tmp/autonomy/logs/cycle-*.log 2>/dev/null | tail -1 | cut -c1-100)
  echo "GATES: not re-run (a worker is in flight); last recorded: ${last:-none}"
else
  echo "GATES: $(bash scripts/checks/gates.sh 2>&1 | tail -1 | cut -c1-120)"
fi
echo "TREE : $(git status --porcelain | wc -l | tr -d ' ') changed path(s) at HEAD $(git log --format='%h' -1)"
echo
echo "WHERE TO LOOK"
echo "· _docs/06-research/  — every Opus report, with provenance"
echo "· _tmp/night/SUMMARY.md — the loop's own summary and its journal"
echo "· _tmp/autonomy/logs/ — one log per cycle, one line per outcome"
