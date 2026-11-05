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

# ---- a behavioural check: what the forced branch READS must be a dispatchable row -----------------
# The previous version of this check asked `state.py get`, which answered — while the forced branch glued
# that JSON behind the id and handed the cycle a "spec" of `{` and an empty title, so every forced item ended
# in needs-spec (measured 2026-11-05). A check that passes while the live path is broken is worse than none.
# So this one runs the call the cycle makes (`pick --row`) and inspects the FIELDS the cycle then uses.
ROWID=$(python3 scripts/autonomy/state.py list | awk -F'\t' 'NR==1{print $1}')
GOT=$(python3 scripts/autonomy/state.py pick --row "$ROWID" 2>/dev/null)
NF_=$(printf '%s' "$GOT" | awk -F'\t' '{print NF}')
SPEC_=$(printf '%s' "$GOT" | awk -F'\t' '{print $2}')
TITLE_=$(printf '%s' "$GOT" | awk -F'\t' '{print $4}')
if [ "$NF_" = 9 ] && [ -f "specs/$SPEC_" ] && [ -n "$TITLE_" ]; then
  pass "the forced row reads as a real plan row: 9 fields, a spec that exists, a title"
else
  bad "the forced row does not read as a plan row (fields=$NF_ spec='$SPEC_' title='$TITLE_')"
fi
# CONTROL: a row that is not in the plan must be REFUSED, not turned into an empty dispatch
if [ "$(python3 scripts/autonomy/state.py pick --row no-such-row-xyz 2>/dev/null)" = "NONE" ]; then
  pass "CONTROL a row that is not in the plan is refused (NONE), not dispatched"
else
  bad "CONTROL an unknown forced row was not refused"
fi
# CONTROL: the cycle must be using that call, not the status JSON
if grep -q 'state.py pick --row' scripts/autonomy/cycle.sh && ! grep -q 'state.py get "\$PII_FORCE_ITEM"' scripts/autonomy/cycle.sh; then
  pass "the cycle forces a row through pick --row and no longer through the status JSON"
else
  bad "the cycle still builds a forced row out of the status JSON"
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
