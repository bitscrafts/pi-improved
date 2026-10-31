#!/usr/bin/env bash
# row-names-its-work.test.sh — the row names its work and its commit, and the claim is checked (spec 326).
# Measured 2026-10-31: all 183 state rows carried exactly {status, attempts, session, last}; the sha lived
# inside free prose ("…the work committed in 593a936..59cfd4c"), the spec only in plan.tsv column 2, and every
# row's id carried an `A` prefix that the plan's own header never defines.
# Runs the REAL state.py and the REAL checkers against scratch files; never runs a cycle, a worker or a gate.
set -u
cd "$(dirname "$0")/../../.." || exit 1
PY="uv run --no-project python scripts/autonomy/state.py"
OK=0
FAILS=0
check(){ if [ "$2" = "$3" ]; then echo "ok $1"; OK=$((OK+1)); else echo "FAIL $1"; echo "  got '$2', want '$3'" >&2; FAILS=$((FAILS+1)); fi; }
T=_tmp/row-names-its-work.$$
mkdir -p "$T/specs"
trap 'find "$T" -depth -delete 2>/dev/null' EXIT
ST=$T/state.json
PL=$T/plan.tsv
SP=$T/specs
printf '%s\n' '# scratch' '## Files' '- `scripts/autonomy/state.py`' '## Exit Criteria' '- [ ] `true`' > "$SP/900-scratch.md"
printf '900\t900-scratch.md\t-\ttitle 900\ttrue\tscripts/autonomy/state.py\t1\n' > "$PL"
row(){ printf '900\t900-scratch.md\t-\ttitle 900\t%s\tscripts/autonomy/state.py\t1\n' "$1" > "$PL"; }
state(){ printf '%s' "$1" > "$ST"; }
ann(){ PII_STATE="$ST" PII_PLAN="$PL" $PY annotate "$@" 2>&1; }
annrc(){ PII_STATE="$ST" PII_PLAN="$PL" $PY annotate "$@" >/dev/null 2>&1; echo $?; }
field(){ PII_STATE="$ST" $PY get 900 | python3 -c "import json,sys;print(json.load(sys.stdin).get('$1','-'))"; }
SHA=$(git rev-parse --verify HEAD 2>/dev/null || true)
# A fresh-gate export is `git archive`: no .git, so no commit can be proven. The test then asserts the
# REFUSAL rather than failing, because that refusal is the correct behaviour there (spec 326 R2).

# (a) annotate writes both, and list shows them — or, in an export with no git, refuses the commit half
state '{}'
if [ -n "$SHA" ]; then
  ann 900 --spec "$SP/900-scratch.md" --commit "$SHA" >/dev/null; rc=$?
  check "(a) annotate exits 0" "$rc" "0"
  check "(a) spec recorded" "$(field spec)" "$SP/900-scratch.md"
  check "(a) commit recorded" "$(field commit)" "$SHA"
  LST=$(PII_STATE="$ST" PII_PLAN="$PL" $PY list)
  check "(a) list carries the spec number" "$(printf '%s' "$LST" | awk -F'\t' '{print $7}')" "900"
  check "(a) list carries the short sha" "$(printf '%s' "$LST" | awk -F'\t' '{print $8}')" "${SHA:0:10}"
else
  echo "note: no .git in this tree (an export) — the commit half is asserted as a refusal"
  check "(a) an unprovable commit is refused" "$(annrc 900 --commit 0123456789abcdef0123456789abcdef01234567)" "1"
  check "(a) the row kept no commit" "$(field commit)" "-"
  ann 900 --spec "$SP/900-scratch.md" >/dev/null; rc=$?
  check "(a) the spec half still lands" "$rc" "0"
  check "(a) spec recorded" "$(field spec)" "$SP/900-scratch.md"
fi

# (b) a sha git does not know is refused, and nothing is written
state '{"900":{"status":"pending"}}'
check "(b) unknown sha refused" "$(annrc 900 --commit deadbeefdeadbeef)" "1"
check "(b) the row kept no commit" "$(field commit)" "-"

# (c) a spec that does not exist is refused
check "(c) missing spec refused" "$(annrc 900 --spec "$SP/nope.md")" "1"
check "(c) the row kept no spec" "$(field spec)" "-"

# (d) a row cannot be annotated with somebody else's spec
check "(d) foreign spec refused" "$(annrc 900 --spec "$SP/900-scratch.md" --spec specs/326-the-row-names-its-work-and-its-commit.md)" "1"

# (e) check-row-truth: a PENDING row is judged against today's spec (that is what it will be dispatched
# against); a DONE row with no commit is counted as historical and never as a pass; a commit git does not know
# is always a violation.
row 'true'
state '{"900":{"status":"pending"}}'
check "(e) a pending faithful row is green" "$(PII_STATE="$ST" PII_PLAN="$PL" PII_SPECS="$SP" bash scripts/autonomy/check-row-truth.sh >/dev/null 2>&1; echo $?)" "0"
row 'true; false'
check "(e) a pending row with a criterion the spec does not have is red" "$(PII_STATE="$ST" PII_PLAN="$PL" PII_SPECS="$SP" bash scripts/autonomy/check-row-truth.sh >/dev/null 2>&1; echo $?)" "1"
row 'true'
state '{"900":{"status":"done"}}'
OUTE=$(PII_STATE="$ST" PII_PLAN="$PL" PII_SPECS="$SP" bash scripts/autonomy/check-row-truth.sh 2>&1)
check "(e) a done row without a commit is not judged" "$(printf '%s' "$OUTE" | grep -c 'historical')" "1"
check "(e) and it is not counted as audited" "$(printf '%s' "$OUTE" | grep -oE '^row-truth: [0-9]+ row' | cut -d' ' -f2)" "0"
state '{"900":{"status":"done","commit":"0123456789abcdef0123456789abcdef01234567"}}'
check "(e) a commit git does not know is red" "$(PII_STATE="$ST" PII_PLAN="$PL" PII_SPECS="$SP" bash scripts/autonomy/check-row-truth.sh >/dev/null 2>&1; echo $?)" "1"

# (f) check-ids: the number is the id from the boundary on, and an older row keeps its alias
printf 'A187\t325-some-spec.md\t-\ttitle\ttrue\tscripts/autonomy/state.py\t1\n' > "$PL"
check "(f) a legacy alias is accepted" "$(PII_PLAN="$PL" PII_ID_BOUNDARY=326 bash scripts/autonomy/check-ids.sh >/dev/null 2>&1; echo $?)" "0"
printf 'A188\t326-the-row-names-its-work-and-its-commit.md\t-\ttitle\ttrue\tscripts/autonomy/state.py\t1\n' > "$PL"
check "(f) an A-id past the boundary is refused" "$(PII_PLAN="$PL" PII_ID_BOUNDARY=326 bash scripts/autonomy/check-ids.sh >/dev/null 2>&1; echo $?)" "1"
printf '326\t326-the-row-names-its-work-and-its-commit.md\t-\ttitle\ttrue\tscripts/autonomy/state.py\t1\n' > "$PL"
check "(f) the spec number as the id is accepted" "$(PII_PLAN="$PL" PII_ID_BOUNDARY=326 bash scripts/autonomy/check-ids.sh >/dev/null 2>&1; echo $?)" "0"

echo "$OK ok, $FAILS FAIL"
[ "$FAILS" -eq 0 ]
