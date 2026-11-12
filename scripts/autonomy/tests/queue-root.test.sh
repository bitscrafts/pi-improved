#!/usr/bin/env bash
# queue-root.test.sh — the queue runs in the repository it was started from (spec 405).
#
# A scratch repository is built under _tmp/: a `git init`, a copy of scripts/autonomy/, a one-row plan.tsv and a fake
# `claude` first on PATH that only answers the probe. Every cycle here runs with DRY=1 and the fake on PATH, so no test
# spends the account or dispatches a worker. The scratch is moved aside when the file is done, never deleted.
#
# The dangerous requirement is R4: a cycle started in the scratch repository must read the scratch plan, take the scratch
# lock and touch nothing under the real repository's _tmp/autonomy/. The real-repository checks compare its state before and
# after the run, not only the scratch output.
#
# Exit: 0 when every check passes. The last line is `PASS: n FAIL: m`.
set -u
REAL=$(cd "$(dirname "$0")/../../.." && pwd)
cd "$REAL" || exit 1
OK=0; FAILS=0
# check <name> <got> <want>: an exact comparison, one PASS or FAIL line per assertion
check(){
  if [ "$2" = "$3" ]; then echo "PASS $1"; OK=$((OK+1)); else echo "FAIL $1"; echo "  got '$2', want '$3'" >&2; FAILS=$((FAILS+1)); fi
}

# The variables a caller may have exported would silently redirect these tests into the live queue.
unset PII_ROOT PII_LOCK PII_STATE PII_PLAN PII_SPECS PII_DRAIN_ROOT PII_CYCLE_LOCKED PII_CONTROL PII_FORCE_ITEM PII_LANE_GRAPH
# A non-interactive shell does not inherit the interactive PATH (python3 lives in /root/.local/bin); see run-tests.sh.
export PATH="/root/.local/bin:/usr/local/cargo/bin:$PATH"

T="$REAL/_tmp/queue-root.$$"
trap 'mkdir -p "$REAL/_tmp/_trash" && mv "$T" "$REAL/_tmp/_trash/queue-root.$$.$(date +%s%N)" 2>/dev/null' EXIT
rm -rf "$T"; mkdir -p "$T/bin"

# The fake claude: answers the probe with PONG55 and records that it was called. Nothing else.
cat >"$T/bin/claude" <<EOF
#!/usr/bin/env bash
echo called >>"$T/claude-calls"
echo '{"type":"result","result":"PONG55","total_cost_usd":0,"modelUsage":{"haiku":{}}}'
EOF
chmod +x "$T/bin/claude"
# The queue's scripts call `python3`. On a box that has only a versioned interpreter (measured 2026-11-11: python3.13, no
# python3) the picker cannot run at all, so the fake bin carries a shim to whichever interpreter is installed.
PY=$(command -v python3 || command -v python3.13 || true)
[ -n "$PY" ] || { echo "queue-root: REFUSED — no python3 on PATH" >&2; exit 2; }
printf '#!/usr/bin/env bash\nexec %s "$@"\n' "$PY" >"$T/bin/python3"
chmod +x "$T/bin/python3"
export PATH="$T/bin:$PATH"

# make_scratch <dir> [--no-plan]: a git repository with a copy of the queue scripts and a one-row plan
make_scratch(){
  local d=$1
  mkdir -p "$d/scripts" "$d/specs"
  cp -R "$REAL/scripts/autonomy" "$d/scripts/autonomy"
  rm -rf "$d/scripts/autonomy/tests" "$d/scripts/autonomy/__pycache__"
  printf '9001\t9001-scratch-row.md\t-\tscratch row for the queue-root test\ttrue\t_tmp/scratch-row.txt\t-\n' > "$d/scripts/autonomy/plan.tsv"
  printf '# Spec: scratch row\n#: depends=- status=pending block=-\n' > "$d/specs/9001-scratch-row.md"
  git -C "$d" init -q
  [ "${2:-}" = "--no-plan" ] && rm -f "$d/scripts/autonomy/plan.tsv"
  return 0
}

S="$T/scratch"
make_scratch "$S"
SR=$(readlink -f "$S")

# (a) env.sh in a scratch copy, sourced from /, exports the scratch repository as PII_ROOT
ROOT_GOT=$(cd / && unset PII_ROOT && . "$S/scripts/autonomy/env.sh" && printf '%s' "${PII_ROOT:-}")
check "root_is_the_repository_the_script_lives_in" "$ROOT_GOT" "$SR"

# (b) CONTROL: a root already in the environment wins
ROOT_ENV=$(cd / && PII_ROOT=/x && . "$S/scripts/autonomy/env.sh" && printf '%s' "$PII_ROOT")
check "CONTROL_a_root_in_the_environment_wins" "$ROOT_ENV" "/x"

# (c) a set-but-blank root is refused loudly, and the message names it
BLANK_OUT=$(cd / && PII_ROOT= bash -c '. "$1"' _ "$S/scripts/autonomy/env.sh" 2>&1); BLANK_RC=$?
check "a_blank_root_is_refused: non-zero exit" "$([ "$BLANK_RC" -ne 0 ] && echo yes || echo no)" "yes"
check "a_blank_root_is_refused: message names PII_ROOT" "$([ "$(printf '%s' "$BLANK_OUT" | grep -c 'PII_ROOT')" -ge 1 ] && echo yes || echo no)" "yes"

# (d) the scratch plan is readable by the queue's own picker: the fixture is sound before any cycle is judged by it
PICK_FIX=$(cd "$S" && python3 scripts/autonomy/state.py pick 2>&1 | head -1)
check "the scratch plan is picked by its own state.py" "$(printf '%s' "$PICK_FIX" | grep -c 'scratch-row')" "1"

# The marker for "nothing under the real _tmp/autonomy changed": created just before the scratch cycle runs.
MARK="$T/mark"; : >"$MARK"; sleep 1; : >"$MARK"
DRY_OUT=$(cd "$REAL" && DRY=1 PATH="$T/bin:$PATH" bash "$S/scripts/autonomy/cycle.sh" 2>&1); DRY_RC=$?

# (e) a dry cycle in the scratch repository, started from the real one, reads the scratch plan and takes the scratch lock
check "a_dry_cycle_in_the_scratch_repo_reads_its_plan_and_its_lock: names the scratch row" "$([ "$(printf '%s' "$DRY_OUT" | grep -c 'scratch-row')" -ge 1 ] && echo yes || echo no)" "yes"
check "a_dry_cycle_in_the_scratch_repo_reads_its_plan_and_its_lock: scratch lock exists" "$([ -e "$S/_tmp/autonomy/lock" ] && echo yes || echo no)" "yes"

# (f) the same run touched nothing under the real repository's _tmp/autonomy/
TOUCHED=$(find "$REAL/_tmp/autonomy" -newer "$MARK" 2>/dev/null | wc -l | tr -d ' ')
check "a_cycle_in_the_scratch_repo_touches_nothing_in_the_real_one" "$TOUCHED" "0"

# (g) CONTROL: the real repository still runs in itself (dry, with the fake claude on PATH: no account is spent)
rm -f "$T/claude-calls"
REAL_OUT=$(cd "$REAL" && DRY=1 PATH="$T/bin:$PATH" bash "$REAL/scripts/autonomy/cycle.sh" 2>&1); REAL_RC=$?
check "CONTROL_the_real_repo_still_runs_in_itself: does not name the scratch row" "$(printf '%s' "$REAL_OUT" | grep -c 'scratch-row')" "0"
check "CONTROL_the_real_repo_still_runs_in_itself: prints an OUTCOME" "$([ "$(printf '%s' "$REAL_OUT" | grep -c 'OUTCOME=')" -ge 1 ] && echo yes || echo no)" "yes"
# The real lock is touched when this run takes it. If another cycle holds it (OUTCOME=locked: measured here, the queue
# runs elsewhere on this volume) the run refuses at the lock, which is the real repository's own answer and spends nothing.
check "CONTROL_the_real_repo_still_runs_in_itself: real lock touched, or held by its own cycle" "$( { [ "$REAL/_tmp/autonomy/lock" -nt "$MARK" ] || printf '%s' "$REAL_OUT" | grep -q 'OUTCOME=locked'; } && echo yes || echo no)" "yes"

# (h) a repository without a plan is refused before the lock and before the probe
# the call log is reset here: the real-repo control above reaches the probe whenever no other cycle holds the lock, and its call must not count against this case
rm -f "$T/claude-calls"
P="$T/noplan"
make_scratch "$P" --no-plan
PLAN_OUT=$(cd "$P" && DRY=1 PATH="$T/bin:$PATH" bash "$P/scripts/autonomy/cycle.sh" 2>&1); PLAN_RC=$?
check "a_repo_without_a_plan_is_refused_before_the_lock: OUTCOME=plan-error" "$(printf '%s' "$PLAN_OUT" | grep -c 'OUTCOME=plan-error')" "1"
check "a_repo_without_a_plan_is_refused_before_the_lock: exit 4" "$PLAN_RC" "4"
check "a_repo_without_a_plan_is_refused_before_the_lock: no lock file created" "$([ -e "$P/_tmp/autonomy/lock" ] && echo yes || echo no)" "no"
check "a_repo_without_a_plan_is_refused_before_the_lock: the fake claude was not called" "$([ -e "$T/claude-calls" ] && echo yes || echo no)" "no"
check "a_repo_without_a_plan_is_refused_before_the_lock: the message names the root it looked at" "$([ "$(printf '%s' "$PLAN_OUT" | grep -c "$(readlink -f "$P")")" -ge 1 ] && echo yes || echo no)" "yes"

# (i) no queue script carries the literal root outside a comment
LIT=$(grep -n -E '^[^#]*/projects/pi-improved' scripts/autonomy/cycle.sh scripts/autonomy/drain.sh scripts/autonomy/hold.sh \
  scripts/autonomy/head-builds.sh scripts/autonomy/pre-commit-builds.sh scripts/autonomy/row-run-criteria.py scripts/autonomy/env.sh 2>/dev/null)
check "no_script_carries_the_literal_root" "$(printf '%s' "$LIT" | grep -c .)" "0"

echo "PASS: $OK FAIL: $FAILS"
[ "$FAILS" -eq 0 ]
