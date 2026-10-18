#!/usr/bin/env bash
# deploy-clean-revision.sh — commit the revision-stamp fixes, install from the clean export, and prove the stamp names
# its commit.
#
# Expected after this run:
#     BUILT FROM EXPORT: <rev> (working-tree changes excluded)
#     revision : <rev>                     ← not "unknown", not "<rev>-dirty"
#
# Run:  bash scripts/deploy-clean-revision.sh
set -uo pipefail
cd /projects/pi-improved || exit 1
. scripts/autonomy/env.sh 2>/dev/null
export XDG_RUNTIME_DIR=/run/user/0
HOST=mvcorrea@10.10.10.217

echo "=== preconditions: nobody may be working ==="
if ! flock -n -E 75 _tmp/autonomy/lock true 2>/dev/null; then
  echo "REFUSING (exit 75): the loop holds its lock. Nothing was changed." >&2
  exit 75
fi
RUNNING=$(ps -eo args | grep -c '[c]ycle\.sh')
echo "cycles running: $RUNNING"
if [ "$RUNNING" -ne 0 ]; then
  echo "REFUSING (exit 75): $RUNNING cycle(s) alive. Nothing was changed." >&2
  exit 75
fi

echo "=== 1. commit the stamp fixes ==="
git add crates/pii-server/build.rs deploy/install-pii.sh scripts/deploy-clean-revision.sh 2>/dev/null
if git commit -q -m "the revision stamp survives a clean export: build.rs prefers PII_BUILD_REVISION because the export has no .git, and the installer hands it in. Two halves of one defect — built in the repo the binary was honest about the working tree (and carried -dirty for it), built in the export it could not name its own commit." 2>/dev/null; then
  echo committed
else
  echo "(nothing new to commit)"
fi
git log --oneline -1
echo "--- tree after commit (should be empty) ---"
git status --porcelain | head -3

echo "=== 2. install from the clean export ==="
bash deploy/install-pii.sh 2>&1 | grep -E 'BUILT FROM EXPORT|INSTALLED|MOVED ASIDE|Finished|CHANNEL|ABORT' || true

echo "=== 3. restart and read the stamp ==="
ssh -o BatchMode=yes "$HOST" 'systemctl --user restart pii && sleep 10 && systemctl --user is-active pii' 2>&1 | tail -1
sleep 6
PII_RUNTIME_DIR=/run/user/0/pii /root/.local/bin/pii health --json 2>/dev/null > /tmp/rev.json

python3 - <<'PY'
import json
try:
    d = json.load(open('/tmp/rev.json'))
except Exception as e:
    print('could not read health:', e)
    raise SystemExit(1)
rev = d.get('build', {}).get('revision') or ''
c = d.get('counts', {}) or {}
print('revision :', rev)
if rev and rev != 'unknown' and 'dirty' not in rev:
    print('VERDICT  : STAMP NAMES ITS COMMIT')
elif 'dirty' in rev:
    print('VERDICT  : still dirty — the export is not being used')
else:
    print('VERDICT  : still cannot name the commit — PII_BUILD_REVISION did not reach the build')
print('cron', c.get('cron_entries'), '| runs', c.get('runs'), '| posts', c.get('channel_posts'))
PY
