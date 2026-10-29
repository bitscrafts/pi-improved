#!/usr/bin/env bash
# row-verify-test.sh — a row's verify column is checked against its spec's ## Exit Criteria (spec 309).
# Each row builds its OWN scratch directory under _tmp/ (never /tmp), writes a fixture plan.tsv and fixture spec,
# and runs the real row-verify.py (and state.py's pick) against them. Prints PASS/FAIL per row, then RESULT.
# Exits non-zero on any failure. Never runs cycle.sh, a gate or a worker; the live plan and state are never read.
set -u
cd "$(dirname "$0")/../../.." || exit 1
ROOT=$(pwd)
RV="$ROOT/scripts/autonomy/row-verify.py"
ST="$ROOT/scripts/autonomy/state.py"
FAILS=0

# scratch: a new empty directory under _tmp/ for one row
scratch(){ mkdir -p "$ROOT/_tmp" && mktemp -d "$ROOT/_tmp/row-verify-XXXXXX"; }
# spec <dir> <criterion>...: specs/S.md with a ## Files entry and one `- [ ]` line per criterion
spec(){
  local d=$1; shift
  mkdir -p "$d/specs"
  { printf '# Spec S\n\n## Files\n- `crates/x/src/a.rs`\n\n## Exit Criteria\n'
    for c in "$@"; do printf -- '- [ ] `%s`\n' "$c"; done; } >"$d/specs/S.md"
}
# plan <dir> <verify-column>: plan.tsv holds one row A1 that names S.md
plan(){ printf 'A1\tS.md\t-\tfixture row\t%s\tcrates/x/src/a.rs\t0\n' "$2" >"$1/plan.tsv"; }
# run <dir> <args...>: row-verify.py against the fixture plan and specs; sets OUT and RC
run(){ local d=$1; shift; OUT=$(PII_PLAN="$d/plan.tsv" PII_SPECS="$d/specs" uv run --no-project python "$RV" "$@" 2>&1); RC=$?; }
# verdict <name> <ok: yes|no> <why>: one PASS or FAIL line per row
verdict(){
  if [ "$2" = yes ]; then echo "PASS $1"; else echo "FAIL $1: $3"; FAILS=$((FAILS+1)); fi
}

C1='grep -q "a b" f'
C2='grep -q "c d" f'
C3='grep -q "e f" f'

D=$(scratch); spec "$D" "$C1" "$C2" "$C3"; plan "$D" "$C1;grep -q \"c  d\" f;$C3"; run "$D" check A1
if [ "$RC" = 1 ] && printf '%s\n' "$OUT" | grep -qF "DIFF 2 spec=$C2 row=grep -q \"c  d\" f"; then
  verdict a-paraphrased-criterion-is-refused yes
else verdict a-paraphrased-criterion-is-refused no "rc=$RC, want 1 and DIFF 2 naming both sides; out: $OUT"; fi

D=$(scratch); spec "$D" "$C1" "$C2" "$C3"; plan "$D" "$C1;$C3"; run "$D" check A1
if [ "$RC" = 1 ] && printf '%s\n' "$OUT" | grep -qxF 'COUNT spec=3 row=2'; then
  verdict a-dropped-criterion-is-refused yes
else verdict a-dropped-criterion-is-refused no "rc=$RC, want 1 and COUNT spec=3 row=2; out: $OUT"; fi

D=$(scratch); spec "$D" "$C1" "$C2" "$C3"; plan "$D" "$C3;$C1;$C2"; run "$D" check A1
if [ "$RC" = 1 ] && printf '%s\n' "$OUT" | grep -q '^DIFF '; then
  verdict a-reordered-row-is-refused yes
else verdict a-reordered-row-is-refused no "rc=$RC, want 1 and at least one DIFF; out: $OUT"; fi

D=$(scratch); spec "$D" "$C1" "$C2" "$C3"; plan "$D" "$C1;$C2;$C3"; run "$D" check A1
if [ "$RC" = 0 ] && [ "$OUT" = "OK A1 3" ]; then
  verdict CONTROL-an-exact-row-is-accepted yes
else verdict CONTROL-an-exact-row-is-accepted no "rc=$RC, want 0 and OK A1 3; out: $OUT"; fi

D=$(scratch); spec "$D" "$C1" "$C2" "$C3"; plan "$D" " $C1 ; $C2 ;$C3  "; run "$D" check A1
if [ "$RC" = 0 ] && [ "$OUT" = "OK A1 3" ]; then
  verdict CONTROL-trailing-whitespace-is-not-a-difference yes
else verdict CONTROL-trailing-whitespace-is-not-a-difference no "rc=$RC, want 0 and OK A1 3; out: $OUT"; fi

D=$(scratch); mkdir -p "$D/specs"
printf '# Spec S\n\n## Files\n- `crates/x/src/a.rs`\n\nProse only, no Exit Criteria block.\n' >"$D/specs/S.md"
plan "$D" "$C1"; run "$D" check A1
case "$OUT" in OK*) silent=yes ;; *) silent=no ;; esac
if [ "$RC" = 2 ] && printf '%s\n' "$OUT" | grep -q '^NO-CRITERIA' && [ "$silent" = no ]; then
  verdict a-spec-without-criteria-is-refused yes
else verdict a-spec-without-criteria-is-refused no "rc=$RC, want 2 and NO-CRITERIA, never OK; out: $OUT"; fi

D=$(scratch); spec "$D" "$C1" 'grep -q A; grep -q B' "$C3"; plan "$D" "$C1;grep -q A; grep -q B;$C3"; run "$D" check A1
if [ "$RC" = 1 ] && printf '%s\n' "$OUT" | grep -qxF 'SPLIT 2: join the commands with && inside one criterion'; then
  verdict a-criterion-holding-the-separator-is-refused yes
else verdict a-criterion-holding-the-separator-is-refused no "rc=$RC, want 1 and SPLIT 2; out: $OUT"; fi

# pick: A1 carries one of the spec's two criteria (refused); A2 carries both (offered). Both rows are pending.
D=$(scratch); spec "$D" "$C1" "$C2"
printf 'A1\tS.md\t-\tshort\t%s\tcrates/x/src/a.rs\t0\nA2\tS.md\t-\tfull\t%s;%s\tcrates/x/src/a.rs\t0\n' "$C1" "$C1" "$C2" >"$D/plan.tsv"
printf '{"A1": {"status": "pending"}, "A2": {"status": "pending"}}' >"$D/state.json"
cp "$D/state.json" "$D/state.before"
PII_PLAN="$D/plan.tsv" PII_STATE="$D/state.json" PII_SPECS="$D/specs" uv run --no-project python "$ST" pick >"$D/pick.out" 2>"$D/pick.err"; PRC=$?
PICKED=$(cut -f1 "$D/pick.out")
if [ "$PRC" = 0 ] && [ "$PICKED" = A2 ] && grep -q '^refused A1' "$D/pick.err" && cmp -s "$D/state.json" "$D/state.before"; then
  verdict pick-refuses-the-mismatched-row-and-offers-the-next yes
else verdict pick-refuses-the-mismatched-row-and-offers-the-next no "rc=$PRC, picked '$PICKED', want A2 with 'refused A1' on stderr and state unchanged; err: $(cat "$D/pick.err")"; fi

if [ "$FAILS" -eq 0 ]; then echo "RESULT: PASS"; exit 0; fi
echo "RESULT: FAIL $FAILS"
exit 1
