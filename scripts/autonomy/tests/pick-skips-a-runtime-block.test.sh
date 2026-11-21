#!/usr/bin/env bash
# pick-skips-a-runtime-block.test.sh — a row parked with `blocked=runtime:...` is skipped, and the queue still advances
# (spec 434). Runs the REAL state.py pick against scratch state, plan and specs; never runs a cycle, a worker or a gate.
#
# Why this file exists: the behaviour was implemented and proven on the live queue on 2026-11-17 (row 425 marked
# blocked, pick dispatched 329 behind it) but the criterion named a test that did not exist, so the row could never
# close. A behaviour proven by hand and not by a test is a behaviour that the next refactor is free to break.
set -u
cd "$(dirname "$0")/../../.." || exit 1
PY="uv run --no-project python scripts/autonomy/state.py"
OK=0
FAILS=0
# check <name> <got> <want>: exact comparison
check(){
  if [ "$2" = "$3" ]; then echo "ok $1"; OK=$((OK+1)); else echo "FAIL $1"; echo "  got '$2', want '$3'" >&2; FAILS=$((FAILS+1)); fi
}
# contains <name> <haystack> <needle>: substring assertion
contains(){
  case "$2" in *"$3"*) echo "ok $1"; OK=$((OK+1)) ;; *) echo "FAIL $1"; echo "  '$2' lacks '$3'" >&2; FAILS=$((FAILS+1)) ;; esac
}
T=_tmp/pick-skips-a-runtime-block.$$
mkdir -p "$T/specs"
trap 'find "$T" -depth -delete 2>/dev/null' EXIT
ST=$T/state.json
PL=$T/plan.tsv
SPECS=$T/specs
# spec <file>: a scratch spec whose Files and Exit Criteria match the rows below, so the row judges admit it
spec(){
  printf '%s\n' '# scratch spec' '#: depends=- status=pending block=-' '' \
    '## Exit Criteria' '- [ ] `true`' '' '## Files' '- `scripts/autonomy/state.py`' > "$SPECS/$1"
}
row(){ printf '%s\t%s\t-\ttitle %s\ttrue\tscripts/autonomy/state.py\t1\n' "$1" "$2" "$1"; }
plan(){
  : >"$PL"
  for a in "$@"; do i=${a%%:*}; s=${a#*:}; spec "$s"; row "$i" "$s" >>"$PL"; done
}
state(){ printf '%s' "$1" >"$ST"; }
pick(){ PII_STATE="$ST" PII_PLAN="$PL" PII_SPECS="$SPECS" $PY pick 2>/dev/null; }
pick_all(){ PII_STATE="$ST" PII_PLAN="$PL" PII_SPECS="$SPECS" $PY pick 2>&1; }

# 1. the property: a runtime-blocked row is not offered, and the next one is
plan A1:s1.md A2:s2.md
state '{"A1":{"status":"pending","blocked":"runtime:lane-not-started-rc=9","last":"lane-not-started-rc=9"},"A2":{"status":"pending"}}'
check a-blocked-row-is-not-offered "$(pick | cut -f1)" "A2"

# 2. the control: without the block, the SAME row is offered first. A test that skipped every row would pass 1 and
#    fail this, which is why it is here.
state '{"A1":{"status":"pending"},"A2":{"status":"pending"}}'
check CONTROL-an-unblocked-row-is-offered "$(pick | cut -f1)" "A1"

# 3. every row blocked: the queue offers NONE and names the block. The assertion is on the PICK RESULT, not on the
#    whole output: the message that names the block contains the row ids, so a "no id anywhere" test would fail a
#    correct implementation - measured here on the first run of this file.
plan A1:s1.md A2:s2.md
state '{"A1":{"status":"pending","blocked":"runtime:lane-not-started-rc=9"},"A2":{"status":"pending","blocked":"runtime:no-podman"}}'
check every-row-blocked-offers-nothing "$(pick)" "NONE"
contains every-row-blocked-names-the-block "$(pick_all)" "blocked"

if [ "$FAILS" -eq 0 ]; then echo "RESULT: PASS ($OK assertions)"; exit 0; fi
echo "RESULT: FAIL $FAILS"
exit 1