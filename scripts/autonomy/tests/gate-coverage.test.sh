#!/usr/bin/env bash
# gate-coverage.test.sh — the queue's gate measures coverage (spec 266): the skip switch is pinned off on the gate command.
# Exits 0 only if every assertion holds. Reads files and sources gate-env.sh only: it never runs cycle.sh or a gate.
set -u
cd "$(dirname "$0")/../../.." || exit 1
GE=scripts/autonomy/gate-env.sh
CYCLE=scripts/autonomy/cycle.sh
[ -f "$GE" ] && . "$GE"
OK=0
FAILS=0
# check <name> <got> <want>: an exact comparison, one ok or FAIL line per assertion
check(){
  if [ "$2" = "$3" ]; then echo "ok $1"; OK=$((OK+1)); else echo "FAIL $1"; echo "  got '$2', want '$3'" >&2; FAILS=$((FAILS+1)); fi
}
# unpinned_lines <file>: the gate command lines that name GATE_STEPS=gate-steps.txt without pinning the skip switch to 0
# (a literal QG_SKIP_COVERAGE=0, or the helper that always prints 0)
unpinned_lines(){
  grep 'GATE_STEPS=gate-steps.txt' "$1" | grep -v -e 'QG_SKIP_COVERAGE=0' -e 'QG_SKIP_COVERAGE=$(gate_skip_coverage)' || true
}

mkdir -p _tmp
T=_tmp/gate-coverage.planted.$$; L=_tmp/gate-coverage.measured.$$; S=_tmp/gate-coverage.skipped.$$; E=_tmp/gate-coverage.empty.$$
trap 'rm -f "$T" "$L" "$S" "$E"' EXIT

# (a) the helper ignores QG_SKIP_COVERAGE=1 from the environment and always prints 0
check "gate_skip_coverage ignores QG_SKIP_COVERAGE from the environment" "$(export QG_SKIP_COVERAGE=1; gate_skip_coverage)" "0"
# (b) cycle.sh pins coverage on for every gate command that names the full list
check "cycle.sh pins coverage on" "$(unpinned_lines "$CYCLE")" ""
# (c) CONTROL for (b): a planted gate line without the variable is flagged by the same predicate
printf '%s\n' '    ( export PATH="/usr/local/cargo/bin:$PATH"; GATE_STEPS=gate-steps.txt CARGO_TARGET_DIR=x timeout 5 bash quality-gate.sh --routine ) >>"$LOG" 2>&1' >"$T"
if [ -n "$(unpinned_lines "$T")" ]; then PLANTED=flagged; else PLANTED=silent; fi
check "CONTROL: the predicate fails on a planted line" "$PLANTED" flagged
# (d) the coverage step skips only when QG_SKIP_COVERAGE is 1: run the step's own if-test under each value
TEST=$(sed -n 's/^coverage-floor|.*\(if \[ "\${QG_SKIP_COVERAGE:-0}" = "1" \]; then\).*/\1/p' gate-steps.txt)
skip_for(){ env QG_SKIP_COVERAGE="$1" bash -c "$TEST echo SKIPPED; fi" 2>/dev/null | grep -c SKIPPED || true; }
check "the step skips only for 1" "$(skip_for 0)/$(skip_for 1)" "0/1"
# (e) coverage_measured reads the roll-up line of a log
printf '%s\n' 'coverage-floor: total 82.80% >= floor 74.00%' >"$L"
printf '%s\n' 'SKIPPED: coverage-floor (QG_SKIP_COVERAGE=1 — the floor was NOT measured in this run,' >"$S"
: >"$E"
check "coverage_measured reads the roll-up line" "$(coverage_measured "$L")/$(coverage_measured "$S")" "yes/no"
# (f) CONTROL for (e): a log with both lines is measured, an empty log is not
cat "$L" "$S" >"$T"
check "CONTROL: both lines is yes, empty is no" "$(coverage_measured "$T")/$(coverage_measured "$E")" "yes/no"

echo "$OK ok, $FAILS FAIL"
[ "$FAILS" -eq 0 ]
