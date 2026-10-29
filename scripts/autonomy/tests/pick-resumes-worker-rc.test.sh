#!/usr/bin/env bash
# pick-resumes-worker-rc.test.sh — a needs-review row whose worker stopped early (last=worker-rc=N) with a session is offered again,
# and the worker-rc path of cycle.sh counts its attempt (spec 312). Part A runs the REAL state.py pick against scratch state and plan
# files. Part B takes the worker-rc block of cycle.sh from the file as text at run time and runs it under set -u with stubs, so a change
# to cycle.sh is followed by this check. Never runs a cycle, a worker or a gate. Exits 0 only if every assertion holds.
# Scratch lives under _tmp/ of this repo and is removed by this script's own trap.
set -u
cd "$(dirname "$0")/../../.." || exit 1
PY="uv run --no-project python scripts/autonomy/state.py"
OK=0
FAILS=0
# check <name> <got> <want>: an exact comparison, one ok or FAIL line per assertion
check(){
  if [ "$2" = "$3" ]; then echo "ok $1"; OK=$((OK+1)); else echo "FAIL $1"; echo "  got '$2', want '$3'" >&2; FAILS=$((FAILS+1)); fi
}
T=_tmp/pick-resumes-worker-rc.$$
mkdir -p "$T"
trap 'find "$T" -depth -delete 2>/dev/null' EXIT
ST=$T/state.json
PL=$T/plan.tsv
# a scratch spec, so the row judges (spec 283, spec 309) read a ## Files list and an ## Exit Criteria that match the rows
SPECS=$T/specs
mkdir -p "$SPECS"
printf '%s\n' '# scratch spec' '## Files' '- `scripts/autonomy/state.py`' '## Exit Criteria' '- [ ] `true`' > "$SPECS/900-scratch.md"
# row <id>: one plan row, seven tab-separated fields (id spec brief title verify paths gate)
row(){ printf '%s\t900-scratch.md\t-\ttitle %s\ttrue\tscripts/autonomy/state.py\t1\n' "$1" "$1"; }
# plan <id...>: writes the plan rows in the order given
plan(){ : >"$PL"; for i in "$@"; do row "$i" >>"$PL"; done; }
# state <json>: writes the state file
state(){ printf '%s' "$1" >"$ST"; }
# pick: what state.py pick prints on stdout for the current scratch files
pick(){ PII_STATE="$ST" PII_PLAN="$PL" PII_SPECS="$SPECS" $PY pick 2>/dev/null; }
# first <tsv>: the first field, the row id (or NONE)
first(){ printf '%s' "$1" | cut -f1; }
# field <tsv> <n>: field n of the TSV line
field(){ printf '%s' "$1" | awk -F'\t' -v n="$2" '{print $n}'; }

# ---- Part A: the real pick ----

# a-worker-rc-row-is-offered: the row is printed with its session as the last field, rc 0
state '{"X1":{"status":"needs-review","last":"worker-rc=1","session":"S1","attempts":2}}'
plan X1
OUT=$(pick); RC=$?
check "a-worker-rc-row-is-offered" "$(first "$OUT")" "X1"
check "a-worker-rc-row-is-offered (session is field 9)" "$(field "$OUT" 9)" "S1"
check "a-worker-rc-row-is-offered (rc 0)" "$RC" "0"

# the-attempts-travel-with-the-row: field 8 is the attempts counter
check "the-attempts-travel-with-the-row" "$(field "$OUT" 8)" "2"

# CONTROL-no-session-is-not-resumable
state '{"X1":{"status":"needs-review","last":"worker-rc=1","session":""}}'
plan X1
check "CONTROL-no-session-is-not-resumable" "$(pick)" "NONE"

# CONTROL-a-dead-login-waits-for-a-person
state '{"X1":{"status":"needs-review","last":"lane-auth-failed","session":"S1"}}'
plan X1
check "CONTROL-a-dead-login-waits-for-a-person" "$(pick)" "NONE"

# CONTROL-a-parked-row-stays-skipped
state '{"X1":{"status":"parked","last":"worker-rc=1","session":"S1"}}'
plan X1
check "CONTROL-a-parked-row-stays-skipped" "$(pick)" "NONE"

# order-is-plan-order: a pending X0 before the worker-rc row, then the worker-rc row first
state '{"X1":{"status":"needs-review","last":"worker-rc=1","session":"S1"}}'
plan X0 X1
check "order-is-plan-order (X0 first)" "$(first "$(pick)")" "X0"
plan X1 X0
check "order-is-plan-order (X1 first)" "$(first "$(pick)")" "X1"

# a-done-row-is-never-offered
state '{"X1":{"status":"done","last":"worker-rc=1","session":"S1"}}'
plan X1
check "a-done-row-is-never-offered" "$(pick)" "NONE"

# pick-does-not-write-the-state: the state file's hash is the same before and after pick
state '{"X1":{"status":"needs-review","last":"worker-rc=1","session":"S1"}}'
plan X1
H1=$(sha256sum "$ST" | cut -d' ' -f1)
pick >/dev/null
H2=$(sha256sum "$ST" | cut -d' ' -f1)
check "pick-does-not-write-the-state" "$H2" "$H1"

# ---- Part B: the worker-rc block of cycle.sh, as text, under set -u with stubs ----

# block <file>: the worker-rc block of cycle.sh, from the line that sets last=worker-rc= to the line printing OUTCOME=worker-rc-
block(){ sed -n '/last=worker-rc=/,/OUTCOME=worker-rc-/p' "$1"; }
# run_block <name> <block-file>: the block as a script under set -u with stubs; argv of the state.py stub goes to $T/<name>.argv,
# the block's rc to $T/<name>.rc
run_block(){
  local n=$1 b=$2 s=$T/$1.sh
  printf '%s\n' 'set -u' "ITEM=X1 ATTEMPTS=0 NEWSID=sid-9 WRC=1" \
    "say() { :; }" "python3() { echo \"\$*\" > \"$T/$n.argv\"; }" > "$s"
  cat "$b" >> "$s"
  bash "$s" >/dev/null 2>&1; echo $? > "$T/$n.rc"
}
BLK=$T/worker-rc.block
block scripts/autonomy/cycle.sh > "$BLK"
if ! grep -q 'OUTCOME=worker-rc-' "$BLK"; then echo "FAIL setup (no worker-rc block in cycle.sh)"; exit 1; fi

# the-cycle-counts-the-attempt: the recorded argv holds attempts=1 and session=sid-9; the block's rc is 8
run_block live "$BLK"
ARGV=$(cat "$T/live.argv" 2>/dev/null)
check "the-cycle-counts-the-attempt (attempts=1 and session=sid-9)" \
  "$(case "$ARGV" in *attempts=1*session=sid-9*|*session=sid-9*attempts=1*) echo yes ;; *) echo no ;; esac)" "yes"
check "the-cycle-counts-the-attempt (rc 8)" "$(cat "$T/live.rc")" "8"

# CONTROL-a-block-that-drops-the-attempt-is-caught: the same block without attempts=$((ATTEMPTS+1)) records no attempts=
sed 's/ attempts=\$((ATTEMPTS+1))//' "$BLK" > "$T/dropped.block"
run_block dropped "$T/dropped.block"
check "CONTROL-a-block-that-drops-the-attempt-is-caught" \
  "$(case "$(cat "$T/dropped.argv" 2>/dev/null)" in *attempts=*) echo has ;; *) echo none ;; esac)" "none"

echo "$OK ok, $FAILS FAIL"
[ "$FAILS" -eq 0 ]
