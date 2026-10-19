#!/usr/bin/env bash
# learn.sh — record ONE queue outcome into pii's own memory, so the product learns from the work it causes.
#
# Why this exists (user directive 2026-10-19: "adding knowledge to pii! those should be implemented asap!"):
# the loop's memory was state.json + git + _docs — all OUTSIDE pii. The product's memory subsystem (bm25 + vector) never
# heard about a single spec implementation, refusal or storm. Specs 183 and 184 already require "learn every outcome to
# memory"; this is the smallest honest implementation of that requirement.
#
# Usage: bash scripts/autonomy/learn.sh A70
#   reads the row's recorded outcome from _tmp/autonomy/state.json and puts one document into pii memory:
#     "row A70 (143-a-run-survives-its-process) status=done attempts=1 commit=3ce83eb — satisfied by 3ce83eb: …"
#
# It NEVER fails a cycle: if the daemon is unreachable it prints a warning and exits 0, because losing a memory write must
# not lose the work that produced it.
set -uo pipefail
cd /projects/pi-improved || exit 1
ITEM=${1:?usage: learn.sh <row-id>}
SOCK_ENV=${PII_RUNTIME_DIR:-/run/user/0/pii}
export PII_RUNTIME_DIR="$SOCK_ENV"
PII=${PII_BIN:-/root/.local/bin/pii}

TEXT=$(python3 - "$ITEM" <<'PY'
import json, subprocess, sys
item = sys.argv[1]
state = json.load(open('_tmp/autonomy/state.json'))
v = state.get(item) if isinstance(state.get(item), dict) else {}
if not v:
    print(''); raise SystemExit
spec = ''
for line in open('scripts/autonomy/plan.tsv'):
    f = line.rstrip('\n').split('\t')
    if f and f[0] == item and len(f) > 1:
        spec = f[1]
commit = subprocess.run(['git', 'log', '-1', '--format=%h %s'], capture_output=True, text=True).stdout.strip()
print(f"row {item} ({spec}) status={v.get('status','?')} attempts={v.get('attempts','-')} "
      f"session={v.get('session','-')} | {v.get('last','')} | newest commit: {commit}")
PY
)
[ -n "$TEXT" ] || { echo "learn: $ITEM has no recorded outcome yet — nothing to learn" >&2; exit 0; }

if ! OUT=$("$PII" memory put "$TEXT" 2>&1); then
  echo "learn: memory unavailable, outcome NOT recorded ($OUT)" >&2
  exit 0
fi
echo "learn: $ITEM -> pii memory $OUT"
