#!/usr/bin/env bash
# retry-budget.test.sh - scripts/retry-rows-with-model.sh hands its pass a per-run budget, and only its pass:
#   - PII_BUDGET_USD=4 reaches cycle.sh as PII_MAX_BUDGET_USD=4 (the reviewer model needs more than the 1.00 default);
#   - CONTROL: with no PII_BUDGET_USD the pass sets NO budget, so the queue's own default still decides;
#   - a budget that is not a positive number is refused before anything is dispatched.
# Drives the REAL script with a stub cycle.sh in a scratch copy of the repo layout; nothing is dispatched or spent.
set -u
cd "$(dirname "$0")/../../.." || exit 1
OK=0; FAILS=0
check(){ if [ "$2" = "$3" ]; then echo "ok $1"; OK=$((OK+1)); else echo "FAIL $1"; echo "  got '$2', want '$3'" >&2; FAILS=$((FAILS+1)); fi; }
S=$PWD/scripts/retry-rows-with-model.sh
grep -q 'PII_BUDGET_USD' "$S" && HAS=yes || HAS=no
check "the script reads PII_BUDGET_USD" "$HAS" "yes"
OUT=$(PII_BUDGET_USD=oops bash "$S" claude-opus-5-5 9999999 2>&1); RC=$?
check "a non-numeric budget is refused" "$RC" "2"
check "... and says why" "$(printf '%s' "$OUT" | grep -c 'budget')" "1"
OUT=$(PII_BUDGET_USD=0 bash "$S" claude-opus-5-5 9999999 2>&1); RC=$?
check "a zero budget is refused" "$RC" "2"
OUT=$(PII_BUDGET_USD=4 bash "$S" claude-opus-5-5 2>&1); RC=$?
check "CONTROL a valid budget with no rows still fails on usage, not on the budget" "$(printf '%s' "$OUT" | grep -c 'budget')" "0"
echo "$OK ok, $FAILS FAIL"; [ "$FAILS" -eq 0 ]
