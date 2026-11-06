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
# The part that was wrong the first time: the budget must REACH the cycle. A stub cycle.sh in a scratch checkout prints
# the variables it receives; the real script runs it for one row. (A test that only checked the refusals passed while the
# dispatch line itself was a syntax error that ran "PII_MAX_BUDGET_USD=4.00" as a command.)
T=_tmp/retry-budget.$$; mkdir -p "$T/scripts/autonomy"; trap 'find "$T" -depth -delete 2>/dev/null' EXIT
cp "$S" "$T/scripts/retry-rows-with-model.sh"
printf '%s\n' '#!/usr/bin/env bash' 'echo "STUB budget=[${PII_MAX_BUDGET_USD:-unset}] model=[${CLAUDE_MODEL:-unset}] force=[${PII_FORCE_ITEM:-unset}]"' 'echo "OUTCOME=done ITEM=1"' > "$T/scripts/autonomy/cycle.sh"
printf '%s\n' '#!/usr/bin/env bash' 'exit 0' > "$T/scripts/autonomy/hold.sh"
printf '%s\n' '#!/usr/bin/env bash' 'exit 0' > "$T/scripts/autonomy/unlisted-changes.sh"
printf '%s\n' 'import sys,json' 'print(json.dumps({"status":"done"}))' > "$T/scripts/autonomy/state.py"
printf '1\t1-x.md\t-\tx\tgreen\t-\n' > "$T/scripts/autonomy/plan.tsv"
run_stub(){ ( cd "$T" && git init -q . 2>/dev/null; sed -i 's|cd /projects/pi-improved|cd "$(dirname "$0")/.."|' scripts/retry-rows-with-model.sh; env "$@" bash scripts/retry-rows-with-model.sh claude-opus-5-5 1 2>&1 ); }
OUT=$(run_stub PII_BUDGET_USD=4.00)
check "a budget reaches the cycle as PII_MAX_BUDGET_USD" "$(printf '%s' "$OUT" | grep -c 'STUB budget=\[4.00\]')" "1"
check "... along with the model and the row, unchanged" "$(printf '%s' "$OUT" | grep -c 'model=\[claude-opus-5-5\] force=\[1\]')" "1"
# the gate runs this with PII_MAX_BUDGET_USD already in its environment; "no budget" must mean none reaches the cycle
# from the script, so the ambient one is cleared for the control (a test that passed by hand and failed in the gate)
OUT=$(run_stub -u PII_MAX_BUDGET_USD PII_BUDGET_USD=)
check "CONTROL no budget: the cycle sees none, so the queue default decides" "$(printf '%s' "$OUT" | grep -c 'STUB budget=\[unset\]')" "1"
check "nothing in the pass failed with a command not found" "$(printf '%s' "$OUT" | grep -c 'command not found')" "0"
echo "$OK ok, $FAILS FAIL"; [ "$FAILS" -eq 0 ]
