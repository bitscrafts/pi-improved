#!/usr/bin/env bash
# land-row.sh: the landing ritual, by the same rule every time — and every refusal it makes is a defect it prevents.
#
# The defect this guards (2026-11-02): six rows were landed by hand, each with the same eleven steps performed from
# memory. Steps performed from memory at 06:00 are steps that get skipped: a criterion not re-run, a path added that
# the row never declared, a row marked done with no commit to point at.
#
# Fixture: a real git repo + the REAL fix-row.sh (symlinked), so the checks under test are the repo's own.
set -uo pipefail
REPO=$(cd "$(dirname "$0")/../../.." && pwd)
SCRIPT=$REPO/scripts/night/land-row.sh
ok=0; bad=0
pass(){ ok=$((ok+1)); echo "ok   $1"; }
fail(){ bad=$((bad+1)); echo "FAIL $1"; }

F=$(mktemp -d "${TMPDIR:-/tmp}/land-XXXXXX"); trap 'rm -rf "$F"' EXIT
ln -s "$REPO/scripts" "$F/scripts"
mkdir -p "$F/specs" "$F/_tmp/autonomy"
git -C "$F" init -q
git -C "$F" config user.email t@t; git -C "$F" config user.name t
: >"$F/.gitkeep"; git -C "$F" add .gitkeep; git -C "$F" commit -qm base
# the fixture's own engine symlink must be COMMITTED, or land-row's rule "no dirt the row does not declare" sees it
# as someone else's work and refuses every case for the wrong reason.
git -C "$F" add scripts >/dev/null 2>&1; git -C "$F" commit -qm "the engine" >/dev/null 2>&1

make_row(){ # $1 id, $2 criterion, $3 paths
  # SIX columns: id, spec, brief(-), title, verify, paths. Five format specs with a spare argument made printf
  # reuse the format and write a second, malformed row — which then failed every check for the wrong reason.
  # SEVEN columns (the last is the gate flag). Six made the plan unparseable, and every case then failed at the
  # row/spec check instead of at the rule it was written for — a fixture bug that looked like ten script bugs.
  printf '%s\t%s\t-\tthe row under test\t%s\t%s\t1\n' "$1" "spec-$1.md" "$2" "$3" >"$F/plan.tsv"
  {
    echo "# Spec: row $1"; echo; echo "## Files"; echo
    printf '%s' "$3" | tr ';' '\n' | sed 's/^/- `/; s/$/`/'
    echo; echo "## Exit Criteria"; echo
    printf '%s' "$2" | tr ';' '\n' | sed 's/^/- [ ] `/; s/$/`/'
  } >"$F/specs/spec-$1.md"
  python3 - "$F" "$1" <<'PY'
import json,sys
root,i=sys.argv[1],sys.argv[2]
json.dump({i:{"status":"needs-review","attempts":3,"commit":None,"session":"s","last":"worker-rc=1"}},
          open(f"{root}/_tmp/autonomy/state.json","w"),indent=1,sort_keys=True)
PY
  # The row, spec and state are scaffolding: commit them, or land-row's "no undeclared dirt" rule (correctly) refuses
  # every case and the suite tests that rule instead of the one it was written for.
  git -C "$F" add plan.tsv specs _tmp >/dev/null 2>&1
  git -C "$F" commit -qm "row $1" >/dev/null 2>&1
}

run(){ PII_LAND_ROOT="$F" PII_PLAN="$F/plan.tsv" bash "$SCRIPT" "$@" 2>&1; }

# 1. a row whose work is finished lands: committed, marked done, commit recorded
make_row 901 'test -f done.txt' 'done.txt;specs/spec-901.md'
: >"$F/done.txt"; git -C "$F" add plan.tsv specs _tmp; git -C "$F" commit -qm "row and spec"
: >"$F/done.txt"
out=$(run 901 -m "Row 901 lands" --no-gate); rc=$?
[ "$rc" -eq 0 ] && pass "a finished row lands (rc=0)" || fail "a finished row lands (rc=$rc): $(printf '%s' "$out" | tail -2 | tr '\n' ' ')"
git -C "$F" log --format='%s' -1 | grep -q 'Row 901 lands' && pass "its commit carries the message" || fail "its commit carries the message"
st=$(python3 -c "import json;d=json.load(open('$F/_tmp/autonomy/state.json'));print(d['901']['status'],d['901'].get('commit'))")
head_sha=$(git -C "$F" rev-parse --short HEAD)
case "$st" in "done $head_sha") pass "the register says done with the commit ($st)" ;; *) fail "the register says done with the commit (got '$st', HEAD is $head_sha)" ;; esac

# 2. a criterion that fails refuses, and nothing is committed
make_row 902 'true' 'done.txt;specs/spec-902.md'
make_row 902 'test -f failing-absent.txt' 'done.txt;specs/spec-902.md'
before=$(git -C "$F" rev-parse HEAD)
out=$(run 902 -m "must not land" --no-gate); rc=$?
[ "$rc" -eq 4 ] && pass "a failing criterion refuses (rc=4)" || fail "a failing criterion refuses (rc=$rc)"
[ "$(git -C "$F" rev-parse HEAD)" = "$before" ] && pass "and nothing was committed" || fail "and nothing was committed"

# 3. a dirty path the row does not declare refuses (the leftover hazard)
make_row 903 'true' 'done.txt;specs/spec-903.md'
: >"$F/someone-elses.txt"
out=$(run 903 -m "would swallow it" --no-gate); rc=$?
[ "$rc" -eq 5 ] && pass "an undeclared dirty path refuses (rc=5)" || fail "an undeclared dirty path refuses (rc=$rc)"
printf '%s' "$out" | grep -q 'someone-elses.txt' && pass "and names the path" || fail "and names the path"
rm -f "$F/someone-elses.txt"

# 4. no message refuses: a commit must say what it did
make_row 904 'true' 'done.txt;specs/spec-904.md'
out=$(run 904 --no-gate); rc=$?
[ "$rc" -eq 2 ] && pass "no message refuses (rc=2)" || fail "no message refuses (rc=$rc)"

# 5. a row that disagrees with its spec refuses, and names the repair
make_row 905 'true' 'done.txt;specs/spec-905.md'
printf -- '- [ ] `true`\n' >>"$F/specs/spec-905.md"   # a second criterion the row does not have
out=$(run 905 -m "x" --no-gate); rc=$?
[ "$rc" -eq 3 ] && pass "row/spec disagreement refuses (rc=3)" || fail "row/spec disagreement refuses (rc=$rc)"
printf '%s' "$out" | grep -q 'fix-row.sh 905' && pass "and names fix-row.sh" || fail "and names fix-row.sh"

# 6. --dry-run changes nothing
make_row 906 'true' 'new906.txt;specs/spec-906.md'; : >"$F/new906.txt"
before=$(git -C "$F" rev-parse HEAD)
out=$(run 906 -m "dry" --no-gate --dry-run); rc=$?
[ "$rc" -eq 0 ] && [ "$(git -C "$F" rev-parse HEAD)" = "$before" ] && pass "--dry-run commits nothing" || fail "--dry-run commits nothing (rc=$rc)"

# 7. CONTROL: the same row without --dry-run does commit — so the dry-run assertion above is not vacuous
out=$(run 906 -m "wet" --no-gate); rc=$?
[ "$rc" -eq 0 ] && [ "$(git -C "$F" rev-parse HEAD)" != "$before" ] && pass "control: without --dry-run it commits" || fail "control: without --dry-run it commits (rc=$rc)"

echo "RESULT: $ok ok, $bad FAIL"
[ "$bad" -eq 0 ]
