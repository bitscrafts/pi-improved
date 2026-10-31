#!/usr/bin/env bash
# sync-verify.test.sh — an open row's verify is derived from its spec; a finished row's text is frozen (spec 327).
# Measured 2026-10-31: 76 of 183 rows could not be reconciled with their spec's Exit Criteria (30 a dropped
# tail). The row IS the gate, so a list that silently loses a criterion means nothing runs it.
set -u
cd "$(dirname "$0")/../../.." || exit 1
PY="uv run --no-project python scripts/autonomy/state.py"
OK=0
FAILS=0
check(){ if [ "$2" = "$3" ]; then echo "ok $1"; OK=$((OK+1)); else echo "FAIL $1"; echo "  got '$2', want '$3'" >&2; FAILS=$((FAILS+1)); fi; }
T=_tmp/sync-verify.$$
mkdir -p "$T/specs"
trap 'find "$T" -depth -delete 2>/dev/null' EXIT
ST=$T/state.json
PL=$T/plan.tsv
SP=$T/specs
printf '%s\n' '# scratch' '## Files' '- `scripts/autonomy/state.py`' '## Exit Criteria' \
  '- [ ] `cmd a`' '- [ ] `cmd b`' '- [ ] `cmd c`' > "$SP/900-scratch.md"
printf '%s\n' '# scratch' '## Exit Criteria' '- [ ] `cmd x`' > "$SP/901-other.md"
row(){ printf '%s\t%s\t-\ttitle\t%s\tscripts/autonomy/state.py\t1\n' "$1" "$2" "${3:-cmd a;cmd b;cmd c}" >> "$PL"; }
: > "$PL"
state(){ printf '%s' "$1" > "$ST"; }
sync(){ PII_STATE="$ST" PII_PLAN="$PL" PII_SPECS="$SP" bash scripts/autonomy/sync-verify.sh "$@"; }
ver(){ printf '%s' "$1" | cut -f5; }
derived(){ PII_STATE="$ST" PII_PLAN="$PL" PII_SPECS="$SP" $PY pick 2>/dev/null | cut -f5; }

# (a) THE POINT: a stale stored list is not what the dispatcher hands the worker
state '{"900":{"status":"pending"}}'
row 900 900-scratch.md 'cmd a;cmd b'
check "(a) pick derives the full criteria set" "$(derived)" 'cmd a;cmd b;cmd c'
check "(a) the file still holds the stale text" "$(ver "$(cat "$PL")")" 'cmd a;cmd b'

# (b) --check names the stale row and writes nothing; the plan is untouched
BEFORE=$(sha256sum "$PL" | cut -d' ' -f1)
OUT=$(sync --check); RC=$?
check "(b) --check is red on a stale row" "$RC" "1"
check "(b) and it names the row" "$(printf '%s' "$OUT" | grep -c '^SYNC-VERIFY: 900:')" "1"
check "(b) and the file did not change" "$(sha256sum "$PL" | cut -d' ' -f1)" "$BEFORE"

# (c) the write path fixes it, atomically, touching only column 5
sync >/dev/null
check "(c) the row in step is green after the write" "$(sync --check >/dev/null 2>&1; echo $?)" "0"
check "(c) the stored text is now the spec's" "$(ver "$(cat "$PL")")" 'cmd a;cmd b;cmd c'
check "(c) the other columns are untouched" "$(cut -f1,2,3,6,7 "$PL" | tr '\t' '|')" "900|900-scratch.md|-|scripts/autonomy/state.py|1"

# (d) FROZEN history: a finished row keeps its own text and is never rewritten, however far the spec moved
state '{"901":{"status":"done"}}'
row 901 901-other.md 'cmd x;cmd y'
sync --check >/dev/null 2>&1; RC=$?
check "(d) a finished row is not a violation" "$RC" "0"
check "(d) its text is still its own" "$(ver "$(grep '^901' "$PL")")" 'cmd x;cmd y'
sync >/dev/null
check "(d) not even the write path touches it" "$(ver "$(grep '^901' "$PL")")" 'cmd x;cmd y'

# (e) a spec with no criteria is refused, not silently emptied
state '{"902":{"status":"pending"}}'
printf '%s\n' '# no criteria here' > "$SP/902-empty.md"
row 902 902-empty.md 'cmd z'
OUT=$(sync); RC=$?
check "(e) an empty spec is a refusal" "$RC" "1"
check "(e) naming the row" "$(printf '%s' "$OUT" | grep -c '902')" "1"
check "(e) and the row keeps its text" "$(ver "$(grep '^902' "$PL")")" 'cmd z'

echo "$OK ok, $FAILS FAIL"
[ "$FAILS" -eq 0 ]
