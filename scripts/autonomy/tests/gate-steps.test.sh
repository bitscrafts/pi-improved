#!/usr/bin/env bash
# gate-steps.test.sh — the queue's gate selector (gate-env.sh) and its wiring in cycle.sh (spec 262).
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
# has <list text> <label>: yes when the label is one whole line of the list
has(){ if grep -qxF -- "$2" <<<"$1"; then echo yes; else echo no; fi; }
# bare_routine_lines <file>: the lines that run the routine gate without GATE_STEPS=gate-steps.txt on the same line
bare_routine_lines(){ grep 'quality-gate.sh --routine' "$1" | grep -v 'GATE_STEPS=gate-steps.txt' || true; }

FULL=$(gate_step_labels gate-steps.txt)
QUICK=$(gate_step_labels gate-steps-quick.txt)
mkdir -p _tmp
T=_tmp/gate-steps.planted.$$; C=_tmp/gate-steps.comment.$$
trap 'rm -f "$T" "$C"' EXIT

# (a) the queue's selector ignores the quick list exported by env.sh
check "gate_steps_file ignores GATE_STEPS from the environment" "$(export GATE_STEPS=gate-steps-quick.txt; gate_steps_file)" "gate-steps.txt"
# (b) the full list names the three steps the quick list lacks
check "the full list names lint-observability" "$(has "$FULL" lint-observability)" yes
check "the full list names test (lib/bins/tests)" "$(has "$FULL" "test (lib/bins/tests)")" yes
check "the full list names coverage-floor" "$(has "$FULL" coverage-floor)" yes
# (c) CONTROL: the quick list lacks lint-observability, so the check above tells the two lists apart
check "CONTROL: the quick list lacks lint-observability" "$(has "$QUICK" lint-observability)" no
# (d) cycle.sh runs the routine gate only with the full list named on the same command
check "cycle.sh gates with the full list" "$(bare_routine_lines "$CYCLE")" ""
# (e) CONTROL for (d): a planted bare line makes the same predicate report a violation
printf '%s\n' '    ( export PATH="/usr/local/cargo/bin:$PATH"; timeout 5 bash quality-gate.sh --routine ) >>"$LOG" 2>&1' >"$T"
if [ -n "$(bare_routine_lines "$T")" ]; then PLANTED=reported; else PLANTED=silent; fi
check "CONTROL: the predicate fails on a planted bare line" "$PLANTED" reported
# (f) a blank or comment-only list has zero labels
printf '\n# nothing gates here\n\n   \n# still nothing\n' >"$C"
check "a comment-only list has zero labels" "$(gate_step_labels "$C" | wc -l | tr -d ' ')" 0

echo "$OK ok, $FAILS FAIL"
[ "$FAILS" -eq 0 ]
