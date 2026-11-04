#!/usr/bin/env bash
# drain-guards.test.sh — the two checks that let the dispatcher stay ON and still never collide.
#
# Operator, 2026-11-04: "cron must always be on!" and "we need some check before the cronjob start or
# peek anything from the queue". So the guard is not a pause: it is a measurement taken at the start of
# every firing. Each rule needs a control that shows the normal path still runs.
set -uo pipefail
cd /projects/pi-improved || exit 1
ok=0; fail=0
pass(){ ok=$((ok+1)); echo "ok   $1"; }
bad(){ fail=$((fail+1)); echo "FAIL $1"; }

OUT=$(mktemp); trap 'rm -f "$OUT"' EXIT
export PII_HOLD=/tmp/hold-absent-$$   # the hold guards are tested by hold.sh's own suite; here we test the two measurements

# The drain must not dispatch anything while a worker is running.
bash -c 'exec -a run-batchC-pii-test sleep 6' &
FAKE=$!
sleep 0.2
PII_HOLD="$PII_HOLD" PII_DRAIN_QUIET=0 MAX_CYCLES=1 bash scripts/autonomy/drain.sh >"$OUT" 2>&1
rc=$?
grep -q 'a worker or cycle is already running' "$OUT" && pass "a running worker makes the drain yield" \
  || bad "a running worker makes the drain yield (rc=$rc, out: $(head -2 "$OUT" | tr '\n' ' '))"
# the guard line itself contains the word "cycle", so the control must look for a DISPATCH, not a word
grep -qE 'OUTCOME=|drain: dispatch|pick:' "$OUT" && bad "CONTROL the drain really did not dispatch" \
  || pass "CONTROL the drain really did not dispatch"
kill "$FAKE" 2>/dev/null; wait "$FAKE" 2>/dev/null

# Uncommitted work at the start of a firing belongs to someone else.
PLANT=_tmp/autonomy/guard-test-plant.txt
mkdir -p "$(dirname "$PLANT")"; printf 'x\n' > "$PLANT"
git add -N "$PLANT" 2>/dev/null || true
PII_HOLD="$PII_HOLD" PII_DRAIN_QUIET=0 MAX_CYCLES=1 bash scripts/autonomy/drain.sh >"$OUT" 2>&1
grep -q 'the tree has uncommitted work' "$OUT" && pass "uncommitted work makes the drain yield" \
  || bad "uncommitted work makes the drain yield (out: $(head -2 "$OUT" | tr '\n' ' '))"
git reset -q -- "$PLANT" 2>/dev/null || true   # never restore --: it rewrites the working tree
rm -f "$PLANT"

# CONTROL: with a clean tree and no hold, the guard does not fire. It is only meaningful on a clean
# tree, so on a dirty one it says SKIP rather than pretending to have passed -- the gate runs this in a
# clean export, where it is a real assertion.
if [ -n "$(git status --porcelain | head -1)" ]; then
  echo "SKIP CONTROL a clean tree is not refused (this tree is dirty: run it in a clean export)"
else
  PII_HOLD="$PII_HOLD" PII_DRAIN_QUIET=0 MAX_CYCLES=0 bash scripts/autonomy/drain.sh >"$OUT" 2>&1
  if grep -qE 'a worker or cycle is already running|the tree has uncommitted work' "$OUT"; then
    bad "CONTROL a clean tree is not refused (out: $(head -2 "$OUT" | tr '\n' ' '))"
  else
    pass "CONTROL a clean tree is not refused"
  fi
fi

echo "----"
echo "$ok ok, $fail FAIL"
[ "$fail" -eq 0 ]
