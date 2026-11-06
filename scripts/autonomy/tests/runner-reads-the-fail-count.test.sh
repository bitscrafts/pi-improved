#!/usr/bin/env bash
# runner-reads-the-fail-count.test.sh - run-tests.sh must not call a test green when the test PRINTS a failure count.
#
# MEASURED 2026-11-06: lane-graph-select printed "PASS: 5 FAIL: 3" and lane-graph-knob "PASS: 10 FAIL: 1", both exited 0,
# and the runner (which read only the exit code) logged "ok" for each, through every gate. The runner is the one place
# that can refuse that, for every present and future test. Drives the REAL runner over a scratch directory of stub tests
# printing the last-line shapes the real tests print.
#   - a printed non-zero count with exit 0 is RED, in each of the shapes ("PASS: 5 FAIL: 3", "9 ok, 3 FAIL", "RESULT: ..");
#   - CONTROLS that must stay green: "PASS: 5 FAIL: 0", "12 ok, 0 FAIL", "18 PASS, 0 FAIL", "ok NEAR-MISS ..." (no count),
#     and the trap that fooled my own scan: "PASS: 5 FAIL: 0" contains "5 FAIL" and is NOT five failures.
set -u
cd "$(dirname "$0")/../../.." || exit 1
OK=0; FAILS=0
check(){ if [ "$2" = "$3" ]; then echo "ok $1"; OK=$((OK+1)); else echo "FAIL $1"; echo "  got '$2', want '$3'" >&2; FAILS=$((FAILS+1)); fi; }
R=$PWD/scripts/autonomy/run-tests.sh
D=$(mktemp -d "$PWD/_tmp/runner-count-XXXXXX") || exit 1
stub(){ mkdir -p "$D/$1/a/b/tests"; printf '#!/usr/bin/env bash\necho "%s"\nexit 0\n' "$2" > "$D/$1/a/b/tests/stub.test.sh"; }
run(){ bash "$R" "$D/$1/a/b/tests" >/dev/null 2>&1; echo $?; }
stub pf  "PASS: 5 FAIL: 3";        check "printed 'PASS: 5 FAIL: 3' with exit 0 is red"   "$(run pf)"  1
stub okf "9 ok, 3 FAIL";           check "printed '9 ok, 3 FAIL' with exit 0 is red"      "$(run okf)" 1
stub res "RESULT: 9 ok, 2 FAIL";   check "printed 'RESULT: 9 ok, 2 FAIL' with exit 0 is red" "$(run res)" 1
stub z1  "PASS: 5 FAIL: 0";        check "CONTROL 'PASS: 5 FAIL: 0' stays green (the '5 FAIL' trap)" "$(run z1)" 0
stub z2  "12 ok, 0 FAIL";          check "CONTROL '12 ok, 0 FAIL' stays green"            "$(run z2)"  0
stub z3  "18 PASS, 0 FAIL";        check "CONTROL '18 PASS, 0 FAIL' stays green"          "$(run z3)"  0
stub z4  "ok NEAR-MISS: the word limit alone"; check "CONTROL a last line with no count stays green" "$(run z4)" 0
mkdir -p "$D/real/a/b/tests"; printf '#!/usr/bin/env bash\necho "PASS: 1 FAIL: 0"\nexit 1\n' > "$D/real/a/b/tests/s.test.sh"
check "CONTROL a non-zero exit is still red" "$(run real)" 1
mkdir -p _tmp/_trash; mv "$D" "_tmp/_trash/runner-count-$(date +%s)-$$"
echo "$OK ok, $FAILS FAIL"
[ "$FAILS" -eq 0 ]
