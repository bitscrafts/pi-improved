#!/usr/bin/env bash
# resume-row-lock.test.sh — scripts/resume-row-at-verify.sh with the cycle lock HELD by someone else:
#   - exits 75 and says why (a cron cycle is in flight), before touching the row or taking the tree hold;
#   - CONTROL: with the lock FREE it gets past that check (proved with a row that does not exist, which it refuses
#     AFTER the lock check, with a different code), so the refusal above is not simply "always refuse".
# Drives the REAL script; PII_LOCK points at a scratch lock so the live cycle lock is never touched.
set -u
cd "$(dirname "$0")/../../.." || exit 1
S=scripts/resume-row-at-verify.sh
OK=0; FAILS=0
check(){ if [ "$2" = "$3" ]; then echo "ok $1"; OK=$((OK+1)); else echo "FAIL $1"; echo "  got '$2', want '$3'" >&2; FAILS=$((FAILS+1)); fi; }
T=_tmp/resume-lock-test.$$; mkdir -p "$T"
trap 'kill $HOLD 2>/dev/null; find "$T" -depth -delete 2>/dev/null' EXIT
LOCK=$PWD/$T/lock; : > "$LOCK"

# (a) the lock is held by another process
flock -o "$LOCK" sleep 60 & HOLD=$!
sleep 0.5
BEFORE=$(python3 scripts/autonomy/state.py get 397 2>/dev/null | md5sum)
OUT=$(PII_LOCK="$LOCK" bash "$S" 397 2>&1); RC=$?
AFTER=$(python3 scripts/autonomy/state.py get 397 2>/dev/null | md5sum)
check "a held cycle lock exits 75" "$RC" "75"
check "... and says a cycle is in flight" "$(printf '%s' "$OUT" | grep -c 'in flight')" "1"
check "... and leaves the row untouched" "$AFTER" "$BEFORE"
check "... and never took the tree hold" "$(printf '%s' "$OUT" | grep -c 'TAKEN')" "0"
kill $HOLD 2>/dev/null; wait $HOLD 2>/dev/null; sleep 0.3

# (b) CONTROL: with the lock FREE the script does not refuse as in flight. It must not START a real cycle here, so it is
# run with a row that the cycle's own dry guard refuses (no such row) AND with PII_DRY=1 style isolation: the script's
# only job in this test is to get past the lock check, which is visible in its first line of output.
OUT=$(PII_LOCK="$LOCK" timeout 20 bash "$S" 99999999 2>&1 | head -3); 
check "CONTROL a free lock is not refused as in flight" "$(printf '%s' "$OUT" | grep -c 'in flight')" "0"
check "CONTROL ... it reaches the tree hold" "$(printf '%s' "$OUT" | grep -c 'TAKEN')" "1"
bash scripts/autonomy/hold.sh release "resume-99999999-$(printf '%s' "$OUT" | grep -o 'resume-99999999-[0-9]*' | head -1 | sed 's/.*-//')" >/dev/null 2>&1
bash scripts/autonomy/hold.sh check 2>&1 | grep -o 'resume-99999999-[0-9]*' | while read h; do bash scripts/autonomy/hold.sh release "$h" >/dev/null 2>&1; done
check "CONTROL ... and the test left no hold behind" "$(bash scripts/autonomy/hold.sh check 2>&1 | tail -1 | grep -c FREE)" "1"

echo "$OK ok, $FAILS FAIL"; [ "$FAILS" -eq 0 ]
