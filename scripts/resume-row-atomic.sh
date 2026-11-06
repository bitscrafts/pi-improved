#!/usr/bin/env bash
# resume-row-atomic.sh <row> - resume ONE row at verify without racing the cron's own cycles.
#   1. take the tree hold so the dispatcher yields from now on;
#   2. wait (bounded, 20 min) for a drain or cycle already in flight to end - it would overwrite the row's state when it finishes;
#   3. set the row to pending/worker-ok, release the hold, and start resume-row-at-verify.sh at once.
# The window between 3's release and the resume's own hold is milliseconds against a 5-minute cron.
set -uo pipefail
cd /projects/pi-improved || exit 1
ROW=${1:?usage: resume-row-atomic.sh <row>}
export PATH=/root/.local/bin:/usr/local/cargo/bin:/usr/local/bin:/usr/bin:/bin:$PATH
OWNER="atomic-$ROW-$$"
bash scripts/autonomy/hold.sh take "$OWNER" 3600 100 || { echo "STOPPED: could not take the hold"; exit 8; }
trap 'bash scripts/autonomy/hold.sh release "$OWNER" >/dev/null 2>&1' EXIT
# A cron firing is drain.sh, which loops up to 8 cycles and checks the hold only ONCE, at its start (measured 2026-11-06:
# one firing re-dispatched the same row every 3 minutes for 25 minutes while this script held the tree). So wait for the
# drain as well as the cycle; a new firing started after the hold is taken yields on its own.
BUSY='scripts/autonomy/(cycle|drain)\.sh'
for _ in $(seq 1 ${WAIT_ROUNDS:-120}); do pgrep -f "$BUSY" >/dev/null || break; sleep 10; done
pgrep -f "$BUSY" >/dev/null && { echo "STOPPED: a drain or cycle is still in flight after the wait"; exit 75; }
python3 scripts/autonomy/state.py set "$ROW" status=pending last=worker-ok attempts=1 >/dev/null
echo "row $ROW set to worker-ok at $(date -u +%H:%M:%SZ)"
bash scripts/autonomy/hold.sh release "$OWNER" >/dev/null 2>&1; trap - EXIT
exec bash scripts/resume-row-at-verify.sh "$ROW"
