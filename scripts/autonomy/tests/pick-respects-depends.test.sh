#!/usr/bin/env bash
# pick-respects-depends.test.sh — a row whose card line's `depends=` is unfinished is not offered (spec 420).
# Runs the REAL state.py pick against scratch state, plan and specs; never runs a cycle, a worker or a gate.
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
# contains <name> <haystack> <needle>: a substring assertion, for the dispatcher's skip line
contains(){
  case "$2" in *"$3"*) echo "ok $1"; OK=$((OK+1)) ;; *) echo "FAIL $1"; echo "  '$2' lacks '$3'" >&2; FAILS=$((FAILS+1)) ;; esac
}
T=_tmp/pick-respects-depends.$$
mkdir -p "$T/specs"
trap 'find "$T" -depth -delete 2>/dev/null' EXIT
ST=$T/state.json
PL=$T/plan.tsv
SPECS=$T/specs
# spec <file> <depends>: a scratch spec whose ## Files and ## Exit Criteria match the rows below, and whose card line
# carries the dependency under test. Without the matching blocks the row judges refuse every row and this file would
# measure the refusal, not the dependency rule.
spec(){
  printf '%s\n' '# scratch spec' "#: depends=$2 status=pending block=-" '' \
    '## Exit Criteria' '- [ ] `true`' '' '## Files' '- `scripts/autonomy/state.py`' > "$SPECS/$1"
}
# row <id> <spec-file>: one plan row (id spec brief title verify paths gate)
row(){ printf '%s\t%s\t-\ttitle %s\ttrue\tscripts/autonomy/state.py\t1\n' "$1" "$2" "$1"; }
# plan <id:spec:depends ...>: one spec and one plan row per argument
plan(){
  : >"$PL"
  for a in "$@"; do
    i=${a%%:*}; rest=${a#*:}; s=${rest%%:*}; d=${rest#*:}
    spec "$s" "$d"
    row "$i" "$s" >>"$PL"
  done
}
state(){ printf '%s' "$1" >"$ST"; }
pick(){ PII_STATE="$ST" PII_PLAN="$PL" PII_SPECS="$SPECS" $PY pick 2>/dev/null; }
pick_err(){ PII_STATE="$ST" PII_PLAN="$PL" PII_SPECS="$SPECS" $PY pick 2>&1 >/dev/null; }
pick_row(){ PII_STATE="$ST" PII_PLAN="$PL" PII_SPECS="$SPECS" $PY pick --row "$1" 2>/dev/null; }
first(){ printf '%s' "$1" | cut -f1; }

# (a) a row whose dependency is done is offered
plan 20:s20.md:- 21:s21.md:20
state '{"20":{"status":"done"}}'
check "(a) a row whose dependency is done is offered" "$(first "$(pick)")" "21"

# (b) CONTROL: an unfinished dependency blocks the row, and the next eligible row is offered instead. Row 20 is in the
# plan and pending (so it blocks) but placed LAST, so the order the pick meets the rows is what this case measures.
plan 21:s21.md:20 22:s22.md:- 20:s20.md:-
state '{"20":{"status":"pending"}}'
check "(b) CONTROL the blocked row is not offered" "$(first "$(pick)")" "22"
contains "(b) the skip line names the row it waits for" "$(pick_err)" "21(waits 20)"

# (c) CONTROL: a spec with no dependency is offered exactly as before
plan 21:s21.md:-
state '{}'
check "(c) CONTROL depends=- is offered" "$(first "$(pick)")" "21"

# (d) CONTROL: a dependency that is not in the plan is skipped and named, never a plan error
plan 21:s21.md:99 22:s22.md:-
state '{}'
check "(d) CONTROL a dependency outside the plan is skipped" "$(first "$(pick)")" "22"
contains "(d) and named as not-in-plan" "$(pick_err)" "21(waits 99:not-in-plan)"

# (e) CONTROL: naming a row on purpose ignores its dependency
plan 21:s21.md:20
state '{"20":{"status":"pending"}}'
check "(e) CONTROL --row offers a row whose dependency is pending" "$(first "$(pick_row 21)")" "21"

# (f) CONTROL: skipping a row touches no state. The state file must be byte-identical after a pick that skipped it.
plan 21:s21.md:20 22:s22.md:-
state '{"20":{"status":"pending"},"21":{"status":"pending","last":"notes","session":"S1"}}'
BEFORE=$(sha256sum "$ST" | cut -d' ' -f1)
pick >/dev/null
AFTER=$(sha256sum "$ST" | cut -d' ' -f1)
check "(f) CONTROL a blocked row's state is untouched" "$AFTER" "$BEFORE"

# (g) CONTROL: a finished dependency set (done + skipped) offers the row
plan 20:s20.md:- 21:s21.md:20,22 22:s22.md:-
state '{"20":{"status":"done"},"22":{"status":"skipped"}}'
check "(g) CONTROL done and skipped dependencies both release the row" "$(first "$(pick)")" "21"

# (h) CONTROL: `needs-review` is unfinished too, so a dependency waiting for a person blocks the row
plan 21:s21.md:20 22:s22.md:- 20:s20.md:-
state '{"20":{"status":"needs-review"}}'
check "(h) CONTROL a needs-review dependency blocks the row" "$(first "$(pick)")" "22"
contains "(h) and is named" "$(pick_err)" "21(waits 20)"

echo "PASS: $OK FAIL: $FAILS"
[ "$FAILS" = 0 ] && [ "$OK" -ge 10 ]
