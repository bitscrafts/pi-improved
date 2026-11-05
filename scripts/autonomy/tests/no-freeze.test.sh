#!/usr/bin/env bash
# no-freeze.test.sh — the night of 2026-11-04/05 must be structurally impossible.
#
# WHAT HAPPENED (measured): row 391's worker finished at 03:11 and left 25 paths in the tree. A worker
# never commits — the cycle does — but nothing resumed the row, so drain.sh's dirty-tree guard refused
# every later firing. 78 firings, 6.5 hours, no commit. And because the lane's "did not start" exit (8)
# was recorded as a worker failure, eight untouched rows carried `worker-rc=8` they never had.
#
# THE INVARIANT UNDER TEST
#   (1) an owned dirty tree RESUMES its row to the gate       (control: an unowned one still refuses)
#   (2) a lane that never started leaves the row PENDING      (control: a real failure goes needs-review)
#   (3) the forced item is the only one that can advance      (control: without it, the pick decides)
set -uo pipefail
cd "$(dirname "$0")/../../.." || exit 1
ROOT=$PWD
ok=0; fail=0
pass(){ ok=$((ok+1)); echo "ok   $1"; }
bad(){ fail=$((fail+1)); echo "FAIL $1"; }

TMP=$(mktemp -d); OUT=$TMP/out
trap 'rm -rf "$TMP"' EXIT

# ---- (1) the drain's ownership question ------------------------------------------------------------
# `leftover.sh owner` is the ONE reader of "whose work is this": OWNER <row> when exactly one open row
# with a session names a dirty path, FOREIGN <n> otherwise. drain.sh must branch on it, not on dirtiness.
if grep -q 'leftover.sh owner' scripts/autonomy/drain.sh; then
  pass "the drain asks whose the dirty tree is"
else
  bad "the drain refuses on a dirty tree without asking whose it is (the 6.5-hour freeze)"
fi

# the refusal must still exist for a tree nobody owns
if grep -q 'no queue row owns' scripts/autonomy/drain.sh; then
  pass "CONTROL an unowned dirty tree is still refused"
else
  bad "CONTROL the unowned-dirty-tree refusal disappeared"
fi

# and the owned branch must actually force the row and allow the tree
if grep -q 'PII_FORCE_ITEM=' scripts/autonomy/drain.sh && grep -q 'ALLOW_DIRTY=1' scripts/autonomy/drain.sh; then
  pass "an owned tree is resumed with the row forced and the tree allowed"
else
  bad "an owned tree is detected but the cycle is not told which row or that the tree is expected"
fi

# ---- (2) "did not start" is not an attempt --------------------------------------------------------
# The lane exits 8 (dirty tree) and 9 (refused before spending). Those are NOT worker failures.
if grep -qE '^ +8\|9\)' scripts/autonomy/cycle.sh; then
  pass "the cycle treats a lane that never started (rc 8/9) as not-an-attempt"
else
  bad "the cycle counts a lane that never started as a worker attempt"
fi
if grep -q 'status=pending' scripts/autonomy/cycle.sh; then
  pass "a not-started dispatch leaves the row pending"
else
  bad "a not-started dispatch does not leave the row pending"
fi
# CONTROL: a real worker failure must still advance the row to needs-review with its rc
if grep -q 'status=needs-review "session=${NEWSID:-}" last=worker-rc=$WRC' scripts/autonomy/cycle.sh; then
  pass "CONTROL a real worker failure still goes needs-review with its rc"
else
  bad "CONTROL the real-failure path was lost while adding the not-started path"
fi

# ---- (3) the forced item is honoured, and only when named -----------------------------------------
if grep -q 'PII_FORCE_ITEM' scripts/autonomy/cycle.sh; then
  pass "the cycle honours the row the dispatcher forces"
else
  bad "the cycle cannot be told which row owns the tree"
fi
if grep -q 'PII_FORCE_ITEM:-' scripts/autonomy/cycle.sh; then
  pass "CONTROL without a forced item the pick still decides"
else
  bad "CONTROL the pick path was replaced instead of guarded"
fi

# ---- a behavioural check, not only a textual one: the forced item really is used ------------------
# Build a throwaway plan + state and ask state.py get, which is what the forced branch reads.
FIX=$TMP/fix; mkdir -p "$FIX/scripts/autonomy"
cp scripts/autonomy/state.py "$FIX/scripts/autonomy/" 2>/dev/null || true
if PII_PLAN="$ROOT/scripts/autonomy/plan.tsv" python3 scripts/autonomy/state.py get 391 >/dev/null 2>&1; then
  pass "state.py get <row> answers for the forced branch"
else
  bad "state.py get <row> does not answer, so the forced branch cannot read its row"
fi


# ---- (4) a finished lane's work is attributable, so it cannot become FOREIGN ------------------------
# The lane captures a session at spawn; if it does not record it against the row, `leftover.sh owner`
# cannot name the row and the tree reads as FOREIGN forever. That is the 03:11 → 09:40 freeze.
if grep -q 'state.py set "$N" "session=$SID"' scripts/run-batchC-pii.sh; then
  pass "the lane records its session against the row at spawn"
else
  bad "the lane captures a session but never records it, so its own work reads as FOREIGN"
fi

# ---- (5) an unowned tree is loud, once ---------------------------------------------------------------
if grep -q 'foreign-alarm' scripts/autonomy/drain.sh; then
  pass "an unowned dirty tree raises an alarm instead of yielding silently"
else
  bad "an unowned dirty tree still blocks the dispatcher silently"
fi
if grep -q 'NOWS - LAST' scripts/autonomy/drain.sh; then
  pass "CONTROL the alarm is rate-limited, so it cannot become the new noise"
else
  bad "CONTROL the alarm has no rate limit"
fi

echo "----"
echo "$ok ok, $fail FAIL"
[ "$fail" = 0 ]
