#!/usr/bin/env bash
# skip-is-not-a-pass.test.sh — spec 395: a skipped gate leaves the row `pending` with attempts unchanged, and a
# missing write dir is WIRING (disk-floor.sh exit 4), not a disk shortage.
set -u
cd "$(dirname "$0")/../../.." || exit 1
OK=0; FAILS=0
check(){ if [ "$2" = "$3" ]; then echo "ok $1"; OK=$((OK+1)); else echo "FAIL $1"; echo "  got '$2', want '$3'" >&2; FAILS=$((FAILS+1)); fi; }
CYCLE=scripts/autonomy/cycle.sh
# the skip branch: from the DRC test to the `exit 13` that ends it
BRANCH=$(awk '/if \[ "\$DRC" -ne 0 \]; then/{on=1} on{print} on&&/exit 13/{exit}' "$CYCLE")

# a_skipped_gate_leaves_the_row_pending
check "a_skipped_gate_leaves_the_row_pending: status=pending" \
  "$(printf %s "$BRANCH" | grep -c 'state.py set "\$ITEM" status=pending')" "1"
check "a_skipped_gate_leaves_the_row_pending: no needs-review" \
  "$(printf %s "$BRANCH" | grep -c 'needs-review')" "0"
check "a_skipped_gate_leaves_the_row_pending: attempts not incremented" \
  "$(printf %s "$BRANCH" | grep -c 'attempts=')" "0"
check "the reason is written as last=gate-skipped:<why>" \
  "$(printf %s "$BRANCH" | grep -c 'last=gate-skipped:')" "1"
check "the OUTCOME line says the row was not advanced" \
  "$(printf %s "$BRANCH" | grep 'OUTCOME=' | grep -ci 'not.advanced\|NOT_ADVANCED\|advanced=no')" "1"

# a_missing_write_dir_is_wiring
T="$PWD/_tmp/skip-is-not-a-pass.$$"; trap 'find "$T" -depth -delete 2>/dev/null' EXIT
mkdir -p "$T"
OUT=$(bash scripts/autonomy/disk-floor.sh 1 "$T" "$T/absent-dir" 2>&1); RC=$?
check "a_missing_write_dir_is_wiring: exit 4" "$RC" "4"
check "a_missing_write_dir_is_wiring: message names the path" \
  "$(printf %s "$OUT" | grep -c "MISSING: $T/absent-dir")" "1"
check "cycle.sh names the missing path as wiring" \
  "$(grep -c 'DRC.*-eq 4\|WIRING' "$CYCLE" | awk '{print ($1>0)?"yes":"no"}')" "yes"

# CONTROL_a_green_gate_still_advances: room on disk -> floor passes (exit 0), no skip
bash scripts/autonomy/disk-floor.sh 1 "$T" >/dev/null 2>&1; check "CONTROL_a_green_gate_still_advances: floor passes" "$?" "0"
# and a real shortage is still exit 1 (LOW), distinct from wiring
bash scripts/autonomy/disk-floor.sh 999999999 "$T" >/dev/null 2>&1; check "CONTROL a real shortage is exit 1" "$?" "1"

echo "$OK ok, $FAILS FAIL"
[ "$FAILS" -eq 0 ]
