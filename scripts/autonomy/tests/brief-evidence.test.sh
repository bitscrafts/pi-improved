#!/usr/bin/env bash
# brief-evidence.test.sh — the corrective brief must never be empty (row 330, measured 2026-11-01).
#
# The defect this pins: cycle.sh's evidence grep matched none of the shapes its own verify step writes, so a
# resumed worker was told "your earlier pass did not survive" with an empty Evidence block, three times, and
# the row parked. The helper is now a file so this can be asserted instead of eyeballed.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../../.." && pwd)
HELPER=$ROOT/scripts/autonomy/brief-evidence.sh
OK=0; FAIL=0
ok()   { OK=$((OK+1));   echo "ok   $1"; }
bad()  { FAIL=$((FAIL+1)); echo "FAIL $1"; }
check(){ if [ "$2" = "$3" ]; then ok "$1"; else bad "$1 (expected '$3', got '$2')"; fi; }

[ -x "$HELPER" ] || { bad "the helper is missing or not executable: $HELPER"; echo "RESULT: $OK ok, $FAIL FAIL"; exit 1; }

SCRATCH=$(cd "$ROOT" && mkdir -p _tmp && mktemp -d "_tmp/brief-evidence-XXXXXX")
trap 'rm -rf "$SCRATCH"' EXIT
trap 'rm -rf "$SCRATCH"' INT TERM

# 1. the shape that really failed: a verify line whose criteria count is short.
# The log is long on purpose. A five-line fixture is rescued by the helper's tail fallback, so it passes even
# when the pattern is broken — measured 2026-11-01: the first mutation stayed GREEN for exactly that reason.
# The verify line sits near the TOP, past the reach of `tail -20`, so only a correct pattern can find it.
{
  echo 'OPEN model=claude-haiku-5-5 cost=0.0002292'
  echo 'LANE-GRAPH: spec'
  echo '== r000000000989 final: "status":"failed" after ~73s'
  echo '== why the run was not accepted:'
  echo '13 verify - 37600+37022 pid=- image=- crit=6/9 fail=bash scripts/autonomy/run-tests.sh 2>&1 | grep -qE "^autonomy tests: [0-9]+ pass rc=-'
  echo '23 unverified - 74624+- pid=- image=- crit=- fail=run.refused{hook=judge, reason=the spec criteria failed} rc=-'
  i=0; while [ "$i" -lt 30 ]; do echo "27 chatter line $i — the worker's own log tail, quoted by everything"; i=$((i+1)); done
} >"$SCRATCH/verify.log"
OUT=$(bash "$HELPER" "$SCRATCH/verify.log")
printf '%s\n' "$OUT" | grep -q 'crit=6/9' && ok "the verify line that carries the failing criterion is quoted" \
  || bad "the verify line was NOT quoted"
printf '%s\n' "$OUT" | grep -q 'run-tests.sh' && ok "the failing criterion's own command is named" \
  || bad "the failing command is not named"
printf '%s\n' "$OUT" | grep -qi 'refused' && ok "the judge's refusal is quoted" || bad "the refusal is not quoted"

# 2. a log with none of the known shapes must still produce a non-empty block (the tail)
printf 'nothing here matches any pattern\nsecond line of an unusual log\n' >"$SCRATCH/plain.log"
OUT2=$(bash "$HELPER" "$SCRATCH/plain.log")
[ -n "$OUT2" ] && ok "an unusual log still yields a non-empty block" || bad "the block was empty"
printf '%s\n' "$OUT2" | grep -q 'second line' && ok "the fallback is the log's own tail" || bad "the fallback is not the tail"

# 3. CONTROL — an absent log must say so, never print nothing
OUT3=$(bash "$HELPER" "$SCRATCH/does-not-exist.log")
[ -n "$OUT3" ] && ok "an absent log is reported, not silent" || bad "an absent log printed nothing"
printf '%s\n' "$OUT3" | grep -q 'no cycle log' && ok "the absent-log line names what is missing" || bad "the absent-log line is unhelpful"

# 4. CONTROL — the gate's own patterns still match (this helper must not lose what the old grep had)
cat >"$SCRATCH/gate.log" <<'EOF'
OUTCOME=gate-failed ITEM=A1
fresh-gate: FAIL — failed steps: test
FAIL: some shell test
EOF
OUT4=$(bash "$HELPER" "$SCRATCH/gate.log")
printf '%s\n' "$OUT4" | grep -q 'failed steps:' && ok "the gate's failed-steps line is still quoted" || bad "failed steps lost"
printf '%s\n' "$OUT4" | grep -q '^OUTCOME=' && ok "the cycle's OUTCOME line is still quoted" || bad "OUTCOME lost"

# 5. the near-miss: a fully green verify line must not be quoted as if it were a failure
printf '13 verify - 1+2 pid=- image=- crit=9/9 fail=- rc=0\nOUTCOME=committed\n' >"$SCRATCH/green.log"
OUT5=$(bash "$HELPER" "$SCRATCH/green.log")
printf '%s\n' "$OUT5" | grep -q 'crit=9/9' && bad "a green verify line was quoted as evidence" \
  || ok "a green verify line (fail=-) is not quoted as a failure"

echo "RESULT: $OK ok, $FAIL FAIL"
[ "$FAIL" -eq 0 ] || exit 1
