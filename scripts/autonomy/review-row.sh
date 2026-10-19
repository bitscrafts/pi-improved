#!/usr/bin/env bash
# review-row.sh <spec-number> — dispatch a REVIEW row: opus, plan mode, read-only, deliverable verified.
#
# User directive 2026-10-19: "an opus analysis must also be put in the queue, positioned before the items that matter".
# A review is work with a brief, a deliverable and a verifier — so it is a row, and it is picked before its dependents.
#
# The spec carries two lines the runner reads:
#   BRIEF: <path>          DELIVERABLE: <path>
set -uo pipefail
cd /projects/pi-improved || exit 1
export PATH="/root/.local/bin:/usr/local/cargo/bin:/usr/local/bin:/usr/bin:/bin:${PATH:-}"
N=${1:?usage: review-row.sh <spec-number>}
SPEC=$(ls specs/"$N"-*.md 2>/dev/null | head -1); [ -n "$SPEC" ] || { echo "no spec for $N"; exit 2; }
BRIEF=$(sed -n 's/^BRIEF:[[:space:]]*//p' "$SPEC" | head -1)
DELIV=$(sed -n 's/^DELIVERABLE:[[:space:]]*//p' "$SPEC" | head -1)
[ -n "$BRIEF" ] && [ -f "$BRIEF" ] || { echo "ABORT: brief '$BRIEF' missing"; exit 3; }
[ -n "$DELIV" ] || { echo "ABORT: no DELIVERABLE line in $SPEC"; exit 4; }
WORKDIR=$(dirname "$BRIEF")
echo "REVIEW   : $SPEC"
echo "BRIEF    : $BRIEF"
echo "DELIVER  : $DELIV"
before=$(git status --porcelain | wc -l)
bash /root/.hermes/skills/research/opus-review/scripts/run-opus-review.sh /projects/pi-improved "$BRIEF" "$WORKDIR" 2>&1 | tail -6
# the deliverable may land in the workdir (plan mode) — copy it where the spec says
# plan mode names its file opus-review-plan-<ts>.md, not 07x-…: take the NEWEST .md in the workdir, whatever its name.
NEWEST=$(ls -t "$WORKDIR"/*.md 2>/dev/null | head -1)
if [ -n "$NEWEST" ] && [ "$NEWEST" != "$BRIEF" ]; then
  mkdir -p "$(dirname "$DELIV")"
  cp -n "$NEWEST" "$DELIV" 2>/dev/null && echo "copied $(basename "$NEWEST") -> $DELIV"
fi
after=$(git status --porcelain | wc -l)
echo "tree delta: $before -> $after dirty paths"
[ -f "$DELIV" ] && echo "DELIVERABLE PRESENT: $DELIV" || { echo "DELIVERABLE MISSING: $DELIV"; exit 5; }
