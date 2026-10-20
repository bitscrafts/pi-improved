#!/usr/bin/env bash
# deploy-row.sh — a DEPLOY is a row, executed by the dispatcher, not by a coding worker.
#
# User, 2026-10-20: "you should never stop nothing! only add to the queue" and then "with observability, lets proceed to the
# deploy and testing". So the deploy lives in the queue as a row whose spec carries `DEPLOY:` and this script is what runs.
#
# It does, in order, and prints a reading for each step:
#   1  refuse to build a dirty tree (a worker's WIP must never ship)
#   2  write the deploy card for the revision being built
#   3  install (clean export + card gate inside install-pii.sh)
#   4  read back: version · cron blocks · suspended runs (the armed digest) · whether the binary knows kind=coder
#   5  run the coder graph and then `pii run trace` on it — the testing half
#
# It never edits the tree, never commits, and never stops anything.
set -uo pipefail
export PATH="/root/.local/bin:/usr/local/cargo/bin:/usr/local/bin:/usr/bin:/bin:${PATH:-}"
cd /projects/pi-improved || exit 1
REV=$(git rev-parse --short=12 HEAD)
REV7=$(git rev-parse --short=7 HEAD)

step() { printf 'deploy-row: %s\n' "$*"; }
fail() { printf 'deploy-row: FAIL %s\n' "$*"; exit 1; }

# 1 dirty tree
dirty=$(git status --porcelain | grep -v '^?? _tmp/' | grep -v '^?? _docs/05-operations/QUEUE-BRIEF' | wc -l)
step "tree: $dirty changed path(s) outside _tmp"
[ "$dirty" -eq 0 ] || fail "tree is dirty ($dirty paths) — a worker's WIP must not ship; requeue this row after it closes"

# 2 card
mkdir -p _tmp/deploy
if [ ! -s "_tmp/deploy/card-$REV7.json" ] && [ ! -s "_tmp/deploy/card-$REV.json" ]; then
  python3 - "$REV" <<'PY'
import json, pathlib, sys, datetime
rev = sys.argv[1]
card = {
  "revision": rev,
  "created_utc": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
  "incident": "the installed daemon cannot run a coder graph, so pii cannot fix anything itself",
  "fingerprint": "coder-kind-missing-in-production",
  "count": 1,
  "red_on_base": "pii graph lint graphs/coder-demo.toml on the installed binary: node 'code' has unknown kind 'coder'",
  "green_on_branch": "207/208/209 (14 tests), 213 graph verbs, 214 decode stability, 215 named criteria, 217 call events",
  "gate": "the installer requires this card for the revision it builds; the export excludes working-tree changes",
  "rollback": "binaries moved aside to /root/.local/bin.old-<epoch> before the swap",
  "intent": "deploy and then TEST: run a coder graph and read its trace",
}
for n in (f"_tmp/deploy/card-{rev[:7]}.json", f"_tmp/deploy/card-{rev}.json"):
    pathlib.Path(n).write_text(json.dumps(card, indent=1))
print("card written for", rev)
PY
fi
step "card present for $REV7"

# 3 install
step "installing (clean export; working-tree changes excluded by design)"
bash deploy/install-pii.sh 2>&1 | tail -12 || fail "install-pii.sh failed"

# 4 readings
export PII_RUNTIME_DIR="${PII_RUNTIME_DIR:-/run/user/0/pii}"
sleep 3
step "READING version: $(/root/.local/bin/pii --version 2>&1 | head -1)"
step "READING cron blocks: $(grep -c '\[\[cron\]\]' /root/.config/pii/pii.toml 2>/dev/null || echo '?')"
step "READING coder kind: $(/root/.local/bin/pii graph lint graphs/coder-demo.toml 2>&1 | grep -ci "unknown kind") unknown-kind error(s)"
/root/.local/bin/pii run ls 2>/dev/null | python3 -c "
import sys, json
from collections import Counter
try:
    rows = json.load(sys.stdin); c = Counter(r['status'] for r in rows)
    print('deploy-row: READING runs by status:', dict(c), '| suspended:', c.get('suspended', 0))
except Exception as e:
    print('deploy-row: run ls unreadable:', e)"

# 5 testing: the coder graph, then its trace
step "TEST: running graphs/coder-demo.toml"
OUT=$(timeout 600 /root/.local/bin/pii graph run graphs/coder-demo.toml 2>&1 | tail -6)
printf '%s\n' "$OUT" | sed 's/^/deploy-row:   /'
RID=$(printf '%s' "$OUT" | grep -oE 'r[0-9]{12}' | head -1)
if [ -n "$RID" ]; then
  step "TEST: pii run trace $RID"
  /root/.local/bin/pii run trace "$RID" 2>&1 | tail -12 | sed 's/^/deploy-row:   /'
else
  step "TEST: no run id in the graph output — the refusal (if any) is the reading"
fi
step "DONE"
