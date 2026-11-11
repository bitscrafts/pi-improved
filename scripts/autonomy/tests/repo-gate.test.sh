#!/usr/bin/env bash
# repo-gate.test.sh — the gate a repository declares in .pii/gate (spec 407).
#
# Each case is a scratch directory under _tmp/ holding (or not holding) a .pii/gate file, and gate-env.sh is sourced with PII_ROOT
# pointing at it. The gate is never run: the checks read the values the selector prints, and the cycle checks refuse a malformed
# file before the lock. The one cycle run is DRY=1 with a fake `claude` first on PATH, so nothing spends the account.
#
# The dangerous requirement is the default: with no .pii/gate, pi-improved must still be gated with exactly the command it had
# (QG_SKIP_COVERAGE=0 included, spec 266), so the first case compares to the old literal byte for byte.
#
# Exit: 0 when every check passes. The last line is `PASS: n FAIL: m`.
set -u
cd "$(dirname "$0")/../../.." || exit 1
REAL=$(pwd)
GE=scripts/autonomy/gate-env.sh
CYCLE=scripts/autonomy/cycle.sh
OK=0; FAILS=0
# check <name> <got> <want>: an exact comparison, one PASS or FAIL line per assertion
check(){
  if [ "$2" = "$3" ]; then echo "PASS $1"; OK=$((OK+1)); else echo "FAIL $1"; echo "  got '$2', want '$3'" >&2; FAILS=$((FAILS+1)); fi
}
# gv <root> <fn>: the stdout of one selector function, sourced with PII_ROOT=<root>
gv(){ (export PII_ROOT="$1"; . "$REAL/$GE"; "$2") 2>/dev/null; }
# refusal <root> <line>: yes when gate_command exits non-zero and its message names .pii/gate and the given line
refusal(){
  local e="$T/refusal.err" rc
  (export PII_ROOT="$1"; . "$REAL/$GE"; gate_command) >/dev/null 2>"$e"; rc=$?
  if [ "$rc" -ne 0 ] && grep -q '\.pii/gate' "$e" && grep -q "line $2" "$e"; then echo yes; else echo "no (rc=$rc: $(tr '\n' ' ' <"$e"))"; fi
}
# covered <root>: true when gate_has_coverage succeeds with PII_ROOT=<root>, false otherwise; missing when the function does not exist
covered(){
  (export PII_ROOT="$1"; . "$REAL/$GE"
   if declare -F gate_has_coverage >/dev/null; then gate_has_coverage && echo true || echo false; else echo missing; fi) 2>/dev/null
}

T="$REAL/_tmp/repo-gate.$$"
trap 'mkdir -p "$REAL/_tmp/_trash" && mv "$T" "$REAL/_tmp/_trash/repo-gate.$$.$(date +%s%N)" 2>/dev/null' EXIT
rm -rf "$T"; mkdir -p "$T"

# gate_file <name> [text...]: a scratch repository whose .pii/gate holds the given lines; prints its path
gate_file(){
  local d="$T/$1"; shift
  mkdir -p "$d/.pii"
  if [ "$#" -gt 0 ]; then printf '%s\n' "$@" >"$d/.pii/gate"; fi
  echo "$d"
}

# ---- the default: no .pii/gate is today's gate, byte for byte ----
NONE=$(gate_file none)
check "without_a_file_the_gate_is_the_one_pi_improved_had" "$(gv "$NONE" gate_command)" "GATE_STEPS=gate-steps.txt QG_SKIP_COVERAGE=0 bash quality-gate.sh --routine"
check "without_a_file_the_build_check_is_cargo_check" "$(gv "$NONE" gate_build_check)" "cargo check -q --workspace"
check "without_a_file_the_floor_is_12000" "$(gv "$NONE" gate_min_free_mb)" "12000"
check "without_a_file_the_timeout_is_3600" "$(gv "$NONE" gate_timeout)" "3600"

# ---- a declared command replaces the gate ----
CMD=$(gate_file command "command = cargo xtask gate")
check "a_declared_command_replaces_the_gate" "$(gv "$CMD" gate_command)" "cargo xtask gate"

# ---- CONTROL: an absent key keeps its default ----
CTL=$(gate_file control "command = cargo xtask gate")
check "CONTROL_an_absent_key_keeps_its_default_floor" "$(gv "$CTL" gate_min_free_mb)" "12000"
check "CONTROL_an_absent_key_keeps_its_default_timeout" "$(gv "$CTL" gate_timeout)" "3600"
check "CONTROL_an_absent_key_keeps_its_default_build_check" "$(gv "$CTL" gate_build_check)" "cargo check -q --workspace"

# ---- declared timeout and floor are read ----
TF=$(gate_file tf "command = true" "timeout_secs = 900" "min_free_mb = 8000")
check "a_declared_timeout_is_read" "$(gv "$TF" gate_timeout)" "900"
check "a_declared_floor_is_read" "$(gv "$TF" gate_min_free_mb)" "8000"

# ---- refusals name the file and the line ----
UNK=$(gate_file unknown "comand = x")
check "an_unknown_key_is_refused_naming_the_line" "$(refusal "$UNK" 1)" "yes"
BAD=$(gate_file floor "command = true" "min_free_mb = lots")
check "a_non_integer_floor_is_refused_naming_the_line" "$(refusal "$BAD" 2)" "yes"
EMPTY=$(gate_file empty "command =")
check "an_empty_command_is_refused" "$(refusal "$EMPTY" 1)" "yes"

# ---- comments and blank lines are ignored ----
CM=$(gate_file comments "# the gate of this repository" "" "   " "# still a comment" "command = cargo xtask gate" "" "# end")
check "comments_and_blank_lines_are_ignored" "$(gv "$CM" gate_command)" "cargo xtask gate"

# ---- cycle.sh holds no literal gate and calls gate_command ----
LIT=$(grep -cE '^[^#]*quality-gate\.sh --routine' "$REAL/$CYCLE")
check "cycle_pins_no_literal_gate" "$LIT" "0"
CALLS=$(grep -qE '^[^#]*gate_command' "$REAL/$CYCLE" && echo yes || echo no)
check "cycle_calls_gate_command" "$CALLS" "yes"

# ---- coverage is demanded only of a gate that has the coverage step ----
mkdir -p "$T/steps"
printf '%s\n' 'formatting|cargo fmt --check' 'test (lib/bins/tests)|cargo test' >"$T/steps/nocov.txt"
printf '%s\n' 'formatting|cargo fmt --check' 'coverage-floor|bash scripts/coverage.sh' >"$T/steps/cov.txt"
NOCOV=$(gate_file nocov "command = true" "steps_file = nocov.txt")
cp "$T/steps/nocov.txt" "$NOCOV/nocov.txt"
WITHCOV=$(gate_file withcov "command = true" "steps_file = cov.txt")
cp "$T/steps/cov.txt" "$WITHCOV/cov.txt"
check "coverage_is_not_demanded_of_a_gate_without_the_step" "$(covered "$NOCOV")" "false"
check "coverage_is_demanded_of_a_gate_with_the_step" "$(covered "$WITHCOV")" "true"
check "CONTROL_coverage_is_demanded_when_there_is_no_file" "$(covered "$NONE")" "true"

# ---- CONTROL: the cycle refuses a malformed file before the lock, and spends nothing ----
# A scratch repository with the queue's scripts, a one-row plan and a bad .pii/gate. The fake claude answers the probe only
# and records that it ran; with the refusal in place the cycle must stop before the probe, so nothing is recorded.
CS="$T/cycle"
mkdir -p "$CS/scripts" "$CS/specs" "$CS/bin"
cp -R "$REAL/scripts/autonomy" "$CS/scripts/autonomy"
rm -rf "$CS/scripts/autonomy/tests" "$CS/scripts/autonomy/__pycache__"
printf '9001\t9001-scratch-row.md\t-\tscratch row for the repo-gate test\ttrue\t_tmp/scratch-row.txt\t-\n' >"$CS/scripts/autonomy/plan.tsv"
printf '# Spec: scratch row\n#: depends=- status=pending block=-\n' >"$CS/specs/9001-scratch-row.md"
mkdir -p "$CS/.pii" "$CS/_tmp"; printf '%s\n' "comand = x" >"$CS/.pii/gate"
git -C "$CS" init -q
cat >"$CS/bin/claude" <<EOF
#!/usr/bin/env bash
echo called >>"$CS/claude-calls"
echo '{"type":"result","result":"PONG55","total_cost_usd":0,"modelUsage":{"haiku":{}}}'
EOF
chmod +x "$CS/bin/claude"
# TMPDIR is the scratch repository's own _tmp: probe.sh's bare mktemp must not depend on the host's /tmp
CYCOUT=$(cd "$CS" && PII_ROOT="$CS" DRY=1 TMPDIR="$CS/_tmp" PATH="$CS/bin:$PATH" timeout 120 bash scripts/autonomy/cycle.sh 2>&1); CYCRC=$?
# exit 4 alone is not the refusal (a probe error exits 4 too), so the outcome, the exit and the named line are checked together
if [ "$CYCRC" -eq 4 ] && printf '%s\n' "$CYCOUT" | grep -q 'OUTCOME=plan-error' && printf '%s\n' "$CYCOUT" | grep -q '\.pii/gate.*line 1'; then REFUSED=yes; else REFUSED="no (rc=$CYCRC)"; fi
check "CONTROL_the_cycle_refuses_a_malformed_file_with_plan_error_exit_4_naming_the_line" "$REFUSED" "yes"
check "CONTROL_the_cycle_refuses_a_malformed_file_before_the_lock" "$([ -e "$CS/_tmp/autonomy/lock" ] && echo taken || echo none)" "none"
check "CONTROL_the_cycle_refuses_a_malformed_file_before_the_probe" "$([ -e "$CS/claude-calls" ] && echo called || echo not-called)" "not-called"

echo "PASS: $OK FAIL: $FAILS"
[ "$FAILS" -eq 0 ]
