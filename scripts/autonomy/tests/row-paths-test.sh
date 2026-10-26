#!/usr/bin/env bash
# row-paths-test.sh — a row's commit paths are checked against the files its spec declares (spec 283).
# Each row builds its OWN scratch directory under _tmp/ (never /tmp), writes a fixture plan.tsv and fixture spec,
# and runs the real row-paths.py (and state.py's pick) against them. Prints PASS/FAIL per row, then RESULT.
# Exits non-zero on any failure. Never runs cycle.sh, a gate or a worker; the live plan and state are never read.
set -u
cd "$(dirname "$0")/../../.." || exit 1
ROOT=$(pwd)
RP="$ROOT/scripts/autonomy/row-paths.py"
ST="$ROOT/scripts/autonomy/state.py"
FAILS=0

# scratch: a new empty directory under _tmp/ for one row
scratch(){ mkdir -p "$ROOT/_tmp" && mktemp -d "$ROOT/_tmp/row-paths-XXXXXX"; }
# fixture <dir> <spec-body> <row-paths>: specs/S.md holds the body, plan.tsv holds one row A1 that names S.md
fixture(){
  mkdir -p "$1/specs"
  printf '%b' "$2" >"$1/specs/S.md"
  printf 'A1\tS.md\t-\tfixture row\t-\t%s\t0\n' "$3" >"$1/plan.tsv"
}
# run_check <dir>: row-paths.py check A1 against the fixture; sets OUT and RC
run_check(){ OUT=$(PII_PLAN="$1/plan.tsv" PII_SPECS="$1/specs" python3 "$RP" check A1 2>&1); RC=$?; }
# verdict <name> <ok: yes|no> <why>: one PASS or FAIL line per row
verdict(){
  if [ "$2" = yes ]; then echo "PASS $1"; else echo "FAIL $1: $3"; FAILS=$((FAILS+1)); fi
}
TWO='# Spec S\n\n## Files\n- `crates/x/src/a.rs`\n- `crates/x/src/b.rs`\n'

D=$(scratch); fixture "$D" "$TWO" "crates/x/src/a.rs"; run_check "$D"
case "$OUT" in *"MISSING crates/x/src/b.rs"*) m=yes ;; *) m=no ;; esac
if [ "$RC" = 1 ] && [ "$m" = yes ]; then verdict a-row-short-a-file-is-refused yes
else verdict a-row-short-a-file-is-refused no "rc=$RC, want 1 and MISSING crates/x/src/b.rs; out: $OUT"; fi

D=$(scratch); fixture "$D" "$TWO" "crates/x/src/a.rs;crates/x/src/b.rs"; run_check "$D"
case "$OUT" in OK\ A1\ 2) ok=yes ;; *) ok=no ;; esac
if [ "$RC" = 0 ] && [ "$ok" = yes ]; then verdict CONTROL-a-row-that-matches-is-accepted yes
else verdict CONTROL-a-row-that-matches-is-accepted no "rc=$RC, want 0 and OK A1 2; out: $OUT"; fi

D=$(scratch); fixture "$D" "$TWO" "crates/x/src/a.rs;crates/x/src/b.rs;crates/x/src/c.rs"; run_check "$D"
case "$OUT" in *"EXTRA crates/x/src/c.rs"*) e=yes ;; *) e=no ;; esac
if [ "$RC" = 1 ] && [ "$e" = yes ]; then verdict an-extra-path-in-the-row-is-refused yes
else verdict an-extra-path-in-the-row-is-refused no "rc=$RC, want 1 and EXTRA crates/x/src/c.rs; out: $OUT"; fi

D=$(scratch); fixture "$D" '# Spec S\n\nProse only, no Files block.\n' "crates/x/src/a.rs"; run_check "$D"
case "$OUT" in NO-FILES*) n=yes ;; *) n=no ;; esac
case "$OUT" in OK*) silent=yes ;; *) silent=no ;; esac
if [ "$RC" = 2 ] && [ "$n" = yes ] && [ "$silent" = no ]; then verdict a-spec-without-a-files-block-is-not-silently-accepted yes
else verdict a-spec-without-a-files-block-is-not-silently-accepted no "rc=$RC, want 2 and NO-FILES, never OK; out: $OUT"; fi

D=$(scratch); fixture "$D" '# Spec S\n\n## Files\n- `crates/x/`\n' "crates/x/"; run_check "$D"
case "$OUT" in NOT-A-FILE*) dir=yes ;; *) dir=no ;; esac
if [ "$RC" = 3 ] && [ "$dir" = yes ]; then verdict a-directory-entry-is-not-a-file-list yes
else verdict a-directory-entry-is-not-a-file-list no "rc=$RC, want 3 and NOT-A-FILE; out: $OUT"; fi

# pick: A1 lists one of its two spec files (refused), A2 lists both (offered); both rows are pending
D=$(scratch); mkdir -p "$D/specs"; printf '%b' "$TWO" >"$D/specs/S.md"
printf 'A1\tS.md\t-\tshort\t-\tcrates/x/src/a.rs\t0\nA2\tS.md\t-\tfull\t-\tcrates/x/src/a.rs;crates/x/src/b.rs\t0\n' >"$D/plan.tsv"
printf '{"A1": {"status": "pending"}, "A2": {"status": "pending"}}' >"$D/state.json"
PII_PLAN="$D/plan.tsv" PII_STATE="$D/state.json" PII_SPECS="$D/specs" python3 "$ST" pick >"$D/pick.out" 2>"$D/pick.err"; PRC=$?
PICKED=$(cut -f1 "$D/pick.out")
if [ "$PRC" = 0 ] && [ "$PICKED" = A2 ] && grep -q '^refused A1' "$D/pick.err"; then verdict CONTROL-a-pick-with-a-refused-row-still-offers-the-next yes
else verdict CONTROL-a-pick-with-a-refused-row-still-offers-the-next no "rc=$PRC, picked '$PICKED', want A2 with 'refused A1' on stderr"; fi

if [ "$FAILS" -eq 0 ]; then echo "RESULT: PASS"; exit 0; fi
echo "RESULT: FAIL $FAILS"
exit 1
