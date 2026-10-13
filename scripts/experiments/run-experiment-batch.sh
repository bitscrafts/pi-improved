#!/usr/bin/env bash
# run-experiment-batch.sh — run the pre-registered experiment's candidates, one per line of the table below.
#
# The seed revision is found by CONTENT, never remembered: the fix commit is the one whose subject starts with the
# spec number, and the base is its parent. (My first attempt passed a closure commit by hand and seeded the tree ON
# the fix — the worker answered "the premise doesn't hold" and was right.)
#
# Each candidate: one uncapped worker (1500 s ceiling), then the harness's mechanical red/green measurement.
# The batch stops early if the account's own severity leaves normal (window.sh), so a spent window parks it.
set -uo pipefail
. "$(dirname "$(readlink -f "$0")")/../autonomy/env.sh"
cd /projects/pi-improved || exit 2

# spec | fingerprint | crate | paths
cat <<'TBL' | while IFS='|' read -r spec fp crate paths; do
133|quarantine-test-race|pii-server|crates
139|verify-failed-metric|pii-learn|crates
137|channel-egress-bounded|pii-channels|crates
135|panicking-node-no-terminal|pii-server|crates
134|fixed-path-store-class|pii-memory|crates
132|forum-channel-posting|pii-channels|crates
141|docs-truth-pass|pii-cli|_docs
TBL
  [ -n "$spec" ] || continue
  if bash scripts/autonomy/window.sh check 2>/dev/null; then
    echo "stopping: $(bash scripts/autonomy/window.sh usage)"
    break
  fi
  fix=$(git log --grep="^$spec:" -1 --format=%h)
  if [ -z "$fix" ]; then echo "[$spec] no fix commit found by content — skipped"; continue; fi
  echo "=== $spec ($fp): fix=$fix base=$(git rev-parse --short "$fix^")"
  bash scripts/experiments/measure-proven-fix.sh "$spec" "$fp" "$fix" "$crate" "$paths"
done
echo "batch done — results: _tmp/autonomy/experiment/results.tsv"
python3 - <<'PY'
import csv, collections
rows = list(csv.DictReader(open('/projects/pi-improved/_tmp/autonomy/experiment/results.tsv'), delimiter='\t'))
proven = [r for r in rows if str(r.get('proof','')).startswith('proven')]
print(f"PROVEN {len(proven)} of {len(rows)} measured candidates")
for r in rows: print(" ", r.get('idx'), r.get('fingerprint'), r.get('proof'), (r.get('note') or '')[:60])
print("DECISION:", "build 151 (>=3 proven)" if len(proven) >= 3 else "stop at 150 + 152 (fewer than 3 proven)")
PY
