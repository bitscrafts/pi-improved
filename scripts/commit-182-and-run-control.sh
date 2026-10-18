#!/usr/bin/env bash
# commit-182-and-run-control.sh
#
# Row 182 closed (852782e) with its WORK uncommitted, so a clean export of HEAD could not compile and the installer
# correctly aborted. This commits the worker's result from an IDLE loop, then runs the deploy negative control.
#
# Preconditions are checked, never assumed: no cycle running, lock free. If an agent is working, refuse — that is the
# rule this session broke once already by running git while row 182's worker was mid-write.
#
# Run:  bash scripts/commit-182-and-run-control.sh
set -uo pipefail
cd /projects/pi-improved || exit 1

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

echo "=== the work the closure left behind ==="
git status --porcelain | head -8

echo "=== commit it: the orchestrator's job, done on an idle loop ==="
git add crates/pii-cli/src/main.rs \
        crates/pii-cli/src/cmd_daemon_flags.rs \
        crates/pii-server/src/daemon/cron.rs \
        scripts/deploy-negative-control.sh \
        scripts/commit-182-and-run-control.sh 2>/dev/null
if git commit -q -m "182's work, committed now that the loop is idle: the daemon's cron and flag handling as separate files, so the two files that blocked the full gate are under the 450-line limit. The row had closed with this uncommitted, which is why a clean export of HEAD would not compile." 2>/dev/null; then
  echo "committed"
else
  echo "(nothing staged)"
fi
git log --oneline -2
echo "--- tree after the commit (should be empty) ---"
git status --porcelain | head -4

echo "=== the deploy negative control ==="
bash scripts/deploy-negative-control.sh 2>&1 | tail -18
