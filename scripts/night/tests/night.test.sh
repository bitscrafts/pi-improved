#!/usr/bin/env bash
# The night's tick, tested without a model, a queue or a clock in the future.
#
# What these assertions are for: the loop's failure modes are (a) seeding the same row twice, which would make two
# rows compete and one dispatch land on a moved tree; (b) running Opus while a worker is in flight, which would
# review a half-written tree; (c) running past the deadline, which is the one thing the operator asked it not to do.
set -uo pipefail
cd "$(dirname "$0")/../../.." || exit 1
N=scripts/night/night.sh
ok=0; bad=0
pass(){ ok=$((ok+1)); echo "ok   $1"; }
fail(){ bad=$((bad+1)); echo "FAIL $1"; }
W=$(mktemp -d "${TMPDIR:-/tmp}/night-XXXXXX"); trap 'rm -rf "$W"' EXIT

# a scratch plan with the real header, so the row format is the real one
head -2 scripts/autonomy/plan.tsv >"$W/plan.tsv"

# 1. seeding appends every row of the tranche
NIGHT_DIR="$W/night" NIGHT_PLAN="$W/plan.tsv" bash "$N" --seed a >"$W/seed1.out" 2>&1
n=$(grep -cv '^#' "$W/plan.tsv")
[ "$n" -eq 4 ] && pass "seeding tranche a appends its 4 rows" || fail "seeding tranche a appends its 4 rows (got $n)"

# 2. seeding twice adds nothing — the guard against two rows competing for one dispatch
NIGHT_DIR="$W/night" NIGHT_PLAN="$W/plan.tsv" bash "$N" --seed a >"$W/seed2.out" 2>&1
n2=$(grep -cv '^#' "$W/plan.tsv")
[ "$n2" -eq 4 ] && pass "seeding twice adds nothing" || fail "seeding twice adds nothing (got $n2)"
grep -q 'already in the plan' "$W/seed2.out" && pass "and it says why" || fail "and it says why"

# 3. the ids are the ones the specs name
for id in 335 336 337 338; do
  grep -q "^$id"$'\t' "$W/plan.tsv" && pass "row $id is in the plan" || fail "row $id is in the plan"
done

# 4. a deadline in the past stops the tick and writes a summary
NIGHT_DIR="$W/n1" NIGHT_PLAN="$W/plan-b.tsv" NIGHT_UNTIL="2020-01-01T00:00:00Z" \
  bash -c 'head -2 scripts/autonomy/plan.tsv >"$1"; NIGHT_DIR="$2" NIGHT_PLAN="$1" NIGHT_UNTIL="2020-01-01T00:00:00Z" bash scripts/night/night.sh' _ "$W/plan-b.tsv" "$W/n1" >"$W/dead.out" 2>&1
grep -q 'deadline' "$W/dead.out" && pass "a past deadline stops the tick" || fail "a past deadline stops the tick ($(tail -1 "$W/dead.out"))"
[ -f "$W/n1/SUMMARY.md" ] && pass "and writes the summary" || fail "and writes the summary"

# 5. the tick's night behaviour: an empty queue with nothing seeded fills tranche a and does not call Opus
NIGHT_DIR="$W/n2" NIGHT_PLAN="$W/plan-c.tsv" bash -c '
  head -2 scripts/autonomy/plan.tsv >"$1"
  NIGHT_DIR="$2" NIGHT_PLAN="$1" bash scripts/night/night.sh' _ "$W/plan-c.tsv" "$W/n2" >"$W/seed-tick.out" 2>&1
grep -q 'seeded tranche a' "$W/seed-tick.out" && pass "an empty queue makes the tick fill tranche a" \
  || fail "an empty queue makes the tick fill tranche a ($(tail -1 "$W/seed-tick.out"))"
! grep -q 'Opus review starting' "$W/seed-tick.out" && pass "and Opus is not started on the same tick" \
  || fail "and Opus is not started on the same tick"
[ "$(grep -cv '^#' "$W/plan-c.tsv")" -eq 4 ] && pass "the rows are in the plan after that tick" \
  || fail "the rows are in the plan after that tick"

# 6. --dry-run changes nothing
before=$(wc -l <"$W/plan.tsv")
NIGHT_DIR="$W/night" NIGHT_PLAN="$W/plan.tsv" bash "$N" --dry-run >"$W/dry.out" 2>&1
after=$(wc -l <"$W/plan.tsv")
[ "$before" = "$after" ] && pass "--dry-run writes nothing" || fail "--dry-run writes nothing"
grep -q 'would:' "$W/dry.out" && pass "and says what it would do" || fail "and says what it would do"

# 7. CONTROL: the tick refuses a second concurrent instance rather than running Opus twice
mkdir -p "$W/n3/.lock"
NIGHT_DIR="$W/n3" NIGHT_PLAN="$W/plan-empty.tsv" bash -c '
  head -2 scripts/autonomy/plan.tsv >"$1"
  NIGHT_DIR="$2" NIGHT_PLAN="$1" bash scripts/night/night.sh' _ "$W/plan-empty.tsv" "$W/n3" >"$W/lock.out" 2>&1
grep -q 'holds the lock' "$W/lock.out" && pass "a held lock makes the tick do nothing" || fail "a held lock makes the tick do nothing ($(tail -1 "$W/lock.out"))"

echo "RESULT: $ok ok, $bad FAIL"
[ "$bad" -eq 0 ]
