#!/usr/bin/env bash
# morning-report.sh — the 07:00 local report. Its STDOUT becomes the Discord message, so it must be self-contained,
# compact and readable on a phone. No model: every line is read from a file or a command, which is the only way a
# report can be trusted at the hour nobody is awake to check it.
set -uo pipefail
cd /projects/pi-improved || exit 1

echo "pii — the night of 02 Nov (07:00 local)"
echo

# 1. the analyses that ran, oldest first, each with the one action it ranked first
mapfile -t REVIEWS < <(ls -1tr _docs/06-research/opus-status-and-path-*.md _docs/06-research/night-review-*.md 2>/dev/null)
echo "OPUS ANALYSES: ${#REVIEWS[@]}"
i=0
for f in "${REVIEWS[@]}"; do
  i=$((i+1))
  when=$(date -u -r "$f" +%H:%M)
  asked=$(awk '/^#+[ ]*(The )?[Ss]ingle next action/{flag=1;next} flag&&NF&&!/^#/{print;exit}' "$f" | cut -c1-190)
  case "$(basename "$f")" in
    opus-status-and-path-*) kind="the audit" ;;
    *)                      kind="a night review" ;;
  esac
  echo "$i. $when  $kind"
  echo "   asked for: ${asked:-（this review did not single out one action）}"
done
echo

# 2. the rows the night closed, with the commit that carries each
echo "CLOSED (the queue's own record):"
ROWS_FILE=${TMPDIR:-/tmp}/pii-rows-$$.tsv
python3 - >"$ROWS_FILE" 2>/dev/null <<'PY' || true
import json
d = json.load(open("_tmp/autonomy/state.json"))
for k in sorted(d, key=lambda x: int(x) if x.isdigit() else 0):
    if k.isdigit() and 335 <= int(k) <= 340:
        r = d[k]
        print(f"{k}\t{r.get('status')}\t{r.get('commit') or '-'}\t{r.get('attempts','?')}")
PY
while IFS=$'\t' read -r id status commit att; do
  [ -n "$id" ] || continue
  subject=$(git log --format='%s' -1 "$commit" 2>/dev/null | cut -c1-70)
  echo "· row $id $status  ${commit}  $subject  (${att} attempt(s))"
done <"$ROWS_FILE"
rm -f "$ROWS_FILE"
echo

# 3. the honest remainder
echo "QUEUE: $(python3 scripts/autonomy/state.py pick 2>/dev/null | head -1 | cut -c1-40)"
if pgrep -f '^bash (.*/)?(scripts/autonomy/cycle|scripts/run-batchC-pii)\.sh' >/dev/null 2>&1; then
  echo "GATES: not re-run (a worker is in flight)"
else
  echo "GATES: $(bash scripts/checks/gates.sh 2>&1 | tail -2 | tr '\n' ' ' | cut -c1-120)"
fi
echo "TREE : $(git status --porcelain | wc -l | tr -d ' ') changed path(s) at HEAD $(git log --format='%h' -1)"
echo "COMMITS tonight: $(git log --since='2026-11-02T05:20:00Z' --oneline | wc -l | tr -d ' ')"
echo
echo "WHERE TO LOOK"
echo "· _docs/06-research/ — every Opus report, with provenance"
echo "· _tmp/night/SUMMARY.md — the loop's own summary and journal"
echo "· _tmp/autonomy/logs/ — one log per cycle"
