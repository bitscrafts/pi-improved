#!/usr/bin/env bash
# pick-resumes-park-cause-repaired.test.sh — a park whose cause was repaired is offered again (spec 323).
# Measured 2026-10-31: A185 was parked after 3 attempts, commit 558759e repaired the cause and the repair was
# written into the row's `last` as prose while status stayed `parked`, so pick skipped it and the loop printed
# OUTCOME=queue-empty with the row still unfinished.
# Runs the REAL state.py against scratch state and plan files; never runs a cycle, a worker or a gate.
set -u
cd "$(dirname "$0")/../../.." || exit 1
PY="uv run --no-project python scripts/autonomy/state.py"
OK=0
FAILS=0
check(){
  if [ "$2" = "$3" ]; then echo "ok $1"; OK=$((OK+1)); else echo "FAIL $1"; echo "  got '$2', want '$3'" >&2; FAILS=$((FAILS+1)); fi
}
T=_tmp/pick-resumes-park-cause-repaired.$$
mkdir -p "$T"
trap 'find "$T" -depth -delete 2>/dev/null' EXIT
ST=$T/state.json
PL=$T/plan.tsv
SPECS=$T/specs
mkdir -p "$SPECS"
printf '%s\n' '# scratch spec' '## Files' '- `scripts/autonomy/state.py`' '## Exit Criteria' '- [ ] `true`' > "$SPECS/900-scratch.md"
row(){ printf '%s\t900-scratch.md\t-\ttitle %s\ttrue\tscripts/autonomy/state.py\t1\n' "$1" "$1"; }
plan(){ : >"$PL"; for i in "$@"; do row "$i" >>"$PL"; done; }
state(){ printf '%s' "$1" >"$ST"; }
pick(){ PII_STATE="$ST" PII_PLAN="$PL" PII_SPECS="$SPECS" $PY pick 2>/dev/null; }
pickerr(){ PII_STATE="$ST" PII_PLAN="$PL" PII_SPECS="$SPECS" $PY pick 2>&1 >/dev/null; }
first(){ printf '%s' "$1" | cut -f1; }
last(){ printf '%s' "$1" | awk -F'\t' '{print $NF}'; }
get(){ PII_STATE="$ST" $PY get "$1"; }

# (a) THE BUG: the repair is an operation that moves the row to needs-review and records why
state '{"X1":{"status":"parked","last":"3-strikes","session":"S1","attempts":3}}'
plan X1
PII_STATE="$ST" $PY repair X1 "the knob test was renamed in 558759e" >/dev/null 2>&1
OUT=$(get X1)
check "(a) repair sets status" "$(printf '%s' "$OUT" | python3 -c 'import json,sys;print(json.load(sys.stdin).get("status"))')" "needs-review"
check "(a) repair records why" "$(printf '%s' "$OUT" | python3 -c 'import json,sys;print(json.load(sys.stdin).get("last","")[:21])')" "park-cause-repaired: "
check "(a) repair keeps the session" "$(printf '%s' "$OUT" | python3 -c 'import json,sys;print(json.load(sys.stdin).get("session"))')" "S1"
check "(a) repair does not touch attempts" "$(printf '%s' "$OUT" | python3 -c 'import json,sys;print(json.load(sys.stdin).get("attempts"))')" "3"

# (b) a repaired park is offered, with its session
check "(b) a repaired park is offered" "$(first "$(pick)")" "X1"
check "(b) with its session last" "$(last "$(pick)")" "S1"
# the picker names the dispatch only when it passed a skipped row over (state.py: `if skipped:`), so the naming
# is checked in that shape: a parked row first, the repaired row second
state '{"X0":{"status":"parked","last":"3-strikes","session":"S0"},"X1":{"status":"needs-review","last":"park-cause-repaired: x","session":"S1"}}'
plan X0 X1
check "(b2) a repaired park behind a parked row is offered" "$(first "$(pick)")" "X1"
check "(b2) and named as dispatching on stderr" "$(pickerr | grep -c -- '> dispatching X1')" "1"

# (c) repair with no session returns the row to pending (a fresh dispatch needs no session)
state '{"X1":{"status":"parked","last":"3-strikes","attempts":2}}'
plan X1
PII_STATE="$ST" $PY repair X1 "cause gone" >/dev/null 2>&1
check "(c) repair without a session -> pending" "$(get X1 | python3 -c 'import json,sys;print(json.load(sys.stdin).get("status"))')" "pending"
check "(c) and it is offered" "$(first "$(pick)")" "X1"

# (d) CONTROL: a plain 3-strikes park is still terminal
state '{"X1":{"status":"parked","last":"3-strikes","session":"S1","attempts":3}}'
plan X1
check "(d) CONTROL a 3-strikes park is NONE" "$(pick)" "NONE"
check "(d) CONTROL and the reason names the waiting row" "$(pickerr | grep -c 'waiting for a person')" "1"

# (e) CONTROL: the A185 shape — marker present, status still parked — is NOT offered
state '{"X1":{"status":"parked","last":"park-cause-repaired: x","session":"S1"}}'
plan X1
check "(e) CONTROL an untranslated marker is NONE" "$(pick)" "NONE"

# (f) CONTROL: a repaired park with an empty session has nothing to resume
state '{"X1":{"status":"needs-review","last":"park-cause-repaired: x","session":""}}'
plan X1
check "(f) CONTROL repaired park with empty session is NONE" "$(pick)" "NONE"

# (g) CONTROL: done stays done
state '{"X1":{"status":"done","last":"park-cause-repaired: x","session":"S1"}}'
plan X1
check "(g) CONTROL a done row is NONE" "$(pick)" "NONE"

# (h) a repaired park does not jump the queue
state '{"X1":{"status":"pending"},"X2":{"status":"needs-review","last":"park-cause-repaired: x","session":"S2"}}'
plan X1 X2
check "(h) a pending row first is picked first" "$(first "$(pick)")" "X1"

# (i) repair of an unknown row fails loudly instead of inventing one
state '{}'
plan X1
PII_STATE="$ST" $PY repair NOSUCH fixed >/dev/null 2>&1
check "(i) repair of an unknown row exits non-zero" "$?" "1"

echo "$OK ok, $FAILS FAIL"
[ "$FAILS" -eq 0 ]
