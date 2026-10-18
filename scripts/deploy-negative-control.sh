#!/usr/bin/env bash
# deploy-negative-control.sh — prove the installer builds from HEAD, not from the working tree.
#
# THE CONTROL: make the tree DIRTY ON PURPOSE, install, and read build.revision.
#     <rev>          →  the fix holds: the binary came from the commit, the dirty tree was excluded
#     <rev>-dirty    →  the fix failed: something still builds from the working tree
#
# A clean-tree run proves nothing — that is exactly how this defect survived three deploys tonight.
#
# Run:  bash scripts/deploy-negative-control.sh
set -uo pipefail
cd /projects/pi-improved || exit 1

# HARD RULE: never touch git while an agent is working.
# Measured 2026-10-18: this control ran `git add`/`git commit` while the loop's row-182 worker was in flight, so the clean
# export of HEAD did not compile (main.rs referenced two files the worker had not committed yet) and the control's verdict
# was meaningless — "FIX FAILED" over an install that had correctly refused and changed nothing.
# The loop holds _tmp/autonomy/lock for the whole cycle, so taking that lock is the only honest way to ask "is anyone in?".
if flock -n -E 75 _tmp/autonomy/lock true 2>/dev/null; then
  :
else
  echo "REFUSING (exit 75): the autonomy loop holds its lock — an agent is working. Nothing was changed." >&2
  echo "  wait for the cycle to end; check with: ps -eo pid,etime,args | grep '[c]ycle.sh'" >&2
  exit 75
fi
if [ -n "$(git status --porcelain | head -1)" ]; then
  echo "REFUSING (exit 75): the working tree is not clean — an agent may be mid-work. Nothing was changed." >&2
  git status --porcelain | head -6 >&2
  exit 75
fi
. scripts/autonomy/env.sh 2>/dev/null
export XDG_RUNTIME_DIR=/run/user/0
HOST=mvcorrea@10.10.10.217
MARK=_docs/06-research/069-opus-the-path-from-deployed-to-production-ready-2026-10-17.md

echo "=== 1. commit the installer fix ==="
git add deploy/install-pii.sh
git commit -q -m "the installer builds from a clean export of HEAD: three consecutive deploys shipped -dirty because cargo ran in the working tree, so the revision a card quotes could include anything uncommitted. The gate has always built from git archive; now the installer does too." \
  || echo "(nothing new to commit)"
git log --oneline -1

echo "=== 2. make the tree dirty ON PURPOSE ==="
printf '\n<!-- negative-control marker, removed after the test -->\n' >> "$MARK"
git status --porcelain | head -4

echo "=== 3. install from the dirty tree ==="
bash deploy/install-pii.sh 2>&1 | grep -E 'BUILT FROM EXPORT|INSTALLED|MOVED ASIDE|Finished|CHANNEL|ABORT' || true

echo "=== 4. restart and read the revision ==="
ssh -o BatchMode=yes "$HOST" 'systemctl --user restart pii && sleep 10 && systemctl --user is-active pii' 2>&1 | tail -1
sleep 6
PII_RUNTIME_DIR=/run/user/0/pii /root/.local/bin/pii health --json 2>/dev/null > /tmp/nc.json

python3 - <<'PY'
import json
try:
    d = json.load(open('/tmp/nc.json'))
except Exception as e:
    print('could not read health:', e)
    raise SystemExit(1)
rev = d.get('build', {}).get('revision') or ''
c = d.get('counts', {}) or {}
print('revision :', rev)
print('CONTROL  :',
      'FIX FAILED — still dirty' if 'dirty' in rev
      else 'FIX HOLDS — built from the commit, the dirty tree was excluded')
print('cron', c.get('cron_entries'), '| runs', c.get('runs'), '| posts', c.get('channel_posts'))
PY

echo "=== 5. remove the deliberate marker (no git checkout, no restore) ==="
python3 - "$MARK" <<'PY'
import sys, pathlib
p = pathlib.Path(sys.argv[1])
lines = [l for l in p.read_text().splitlines() if 'negative-control marker' not in l]
p.write_text('\n'.join(lines).rstrip() + '\n')
print('marker removed')
PY
echo "--- tree after cleanup (should be empty) ---"
git status --porcelain | head -3
