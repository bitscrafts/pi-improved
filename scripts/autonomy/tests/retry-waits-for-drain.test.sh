#!/usr/bin/env bash
# retry-waits-for-drain.test.sh - a retry pass does not start under a running cron drain (measured 2026-11-06: a pass
# started under one was refused at the cycle lock with nothing done, and a drain checks the tree hold only at its start).
#   - a process matching the busy pattern + WAIT_ROUNDS=1 -> the pass exits 75 and says a drain or cycle is in flight;
#   - CONTROL: no such process -> the pass gets past the wait (it stops later, on the budget check, with a different code).
# Drives the REAL script. The busy pattern is a unique string so the real cron cannot change the answer.
set -u
cd "$(dirname "$0")/../../.." || exit 1
OK=0; FAILS=0
check(){ if [ "$2" = "$3" ]; then echo "ok $1"; OK=$((OK+1)); else echo "FAIL $1"; echo "  got '$2', want '$3'" >&2; FAILS=$((FAILS+1)); fi; }
S=$PWD/scripts/retry-rows-with-model.sh
TAG="pii-busy-$$-$RANDOM"
# The hold file is the script's real one, and inside the gate something else (the cycle) holds it; the test's two
# runs must not race the operator's hold. It gets its own scratch hold.
export PII_HOLD="$PWD/_tmp/retry-hold-test.$$"
export PII_BUSY_PATTERN="$TAG" WAIT_ROUNDS=1
# a stand-in for a running drain: any process whose command line carries the tag
bash -c "exec -a '$TAG' sleep 60" & BUSYPID=$!
sleep 0.3
OUT=$(bash "$S" claude-opus-5-5 9999999 2>&1); RC=$?
check "a running drain makes the pass wait, then stop" "$RC" "75"
check "... and say why" "$(printf '%s' "$OUT" | grep -c 'drain or cycle is still in flight')" "1"
kill "$BUSYPID" 2>/dev/null; wait "$BUSYPID" 2>/dev/null
rm -f "$PII_HOLD"
OUT=$(bash "$S" claude-opus-5-5 9999999 2>&1); RC=$?
check "CONTROL: nothing running -> the pass is not stopped by the wait" "$(printf '%s' "$OUT" | grep -c 'still in flight')" "0"
echo "$OK ok, $FAILS FAIL"
[ "$FAILS" -eq 0 ]
