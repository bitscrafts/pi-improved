#!/usr/bin/env bash
# pick-resumes-window-closed.test.sh — a needs-review row parked by a closed window is offered again with its session (spec 280).
# Runs the REAL state.py pick against scratch state and plan files; never runs a cycle, a worker or a gate.
# Exits 0 only if every assertion holds. Scratch lives under _tmp/ and is removed by this script's own trap.
set -u
cd "$(dirname "$0")/../../.." || exit 1
PY="uv run --no-project python scripts/autonomy/state.py"
OK=0
FAILS=0
# check <name> <got> <want>: an exact comparison, one ok or FAIL line per assertion
check(){
  if [ "$2" = "$3" ]; then echo "ok $1"; OK=$((OK+1)); else echo "FAIL $1"; echo "  got '$2', want '$3'" >&2; FAILS=$((FAILS+1)); fi
}
T=_tmp/pick-resumes-window-closed.$$
mkdir -p "$T"
trap 'find "$T" -depth -delete 2>/dev/null' EXIT
ST=$T/state.json
PL=$T/plan.tsv
# a scratch spec, so the row judges (spec 283, spec 309) read a ## Files list and an ## Exit Criteria that match the rows.
# Before this, the rows named the real spec 280 and the paths column "-", so every row was refused (NO-FILES) and this
# whole fixture read 6 ok / 5 FAIL while passing nothing through the rule it tests (measured 2026-10-29).
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
# last <tsv>: the last field, the session of a dispatched row
last(){ printf '%s' "$1" | awk -F'\t' '{print $NF}'; }

# (a) THE BUG: a window-parked row is offered, with its session as the last field
state '{"X1":{"status":"needs-review","last":"window-closed","session":"S1"}}'
plan X1
OUT=$(pick)
check "(a) a window-parked row is offered" "$(first "$OUT")" "X1"
check "(a) its session is the last field" "$(last "$OUT")" "S1"

# (b) CONTROL: needs-review with another last, even with a session, stays skipped
state '{"X1":{"status":"needs-review","last":"lane-auth-failed","session":"S1"}}'
plan X1
check "(b) CONTROL a needs-review row with last=lane-auth-failed is NONE" "$(pick)" "NONE"

# (c) CONTROL: window-closed with an EMPTY session has nothing to resume
state '{"X1":{"status":"needs-review","last":"window-closed","session":""}}'
plan X1
check "(c) CONTROL window-closed with an empty session is NONE" "$(pick)" "NONE"

# (d) CONTROL: a parked row stays skipped even with window-closed and a session
state '{"X1":{"status":"parked","last":"window-closed","session":"S1"}}'
plan X1
check "(d) CONTROL a parked row is NONE" "$(pick)" "NONE"

# (e) order: plan order decides, whichever side the window-parked row sits on
state '{"X1":{"status":"needs-review","last":"window-closed","session":"S1"}}'
plan X0 X1
check "(e) a pending row before the window-parked row is picked first" "$(first "$(pick)")" "X0"
plan X1 X0
check "(e) a window-parked row before a pending row is picked first" "$(first "$(pick)")" "X1"

# (f) after the window's worker is recorded as finished, the same row is skipped again
state '{"X1":{"status":"needs-review","last":"window-closed","session":"S1"}}'
plan X1
PII_STATE="$ST" $PY set X1 last=worker-ok
check "(f) after last=worker-ok the same row is NONE" "$(pick)" "NONE"

# (g) CONTROL: a done row is never picked
state '{"X1":{"status":"done","last":"window-closed","session":"S1"}}'
plan X1
check "(g) CONTROL a done row is NONE" "$(pick)" "NONE"

# (h) a window-parked row first and another needs-review row second: only the first is offered
state '{"X1":{"status":"needs-review","last":"window-closed","session":"S1"},"X2":{"status":"needs-review","last":"lane-auth-failed","session":"S2"}}'
plan X1 X2
OUT=$(pick)
check "(h) the window-parked row first is offered" "$(first "$OUT")" "X1"
check "(h) the other needs-review row is not offered" "$(pick | grep -c '^X2')" "0"

echo "$OK ok, $FAILS FAIL"
[ "$FAILS" -eq 0 ]
