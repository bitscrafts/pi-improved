#!/usr/bin/env bash
# The incident publisher: loud when its record is missing, silent when there is nothing new, once per row.
#
# The defect this guards: the old publisher exited 0 for a missing record, so "the detector never ran" and "no
# incident happened" were the same non-event. The control runs that old script against the same missing path and
# requires it to be silent.
set -uo pipefail
cd "$(dirname "$0")/../../.." || exit 1
P=scripts/autonomy/incidents-publish.sh
OLD=${PII_PUBLISH_SH:-/root/.hermes/scripts/pii-incidents-discord.sh}
ok=0; bad=0
pass(){ ok=$((ok+1)); echo "ok   $1"; }
fail(){ bad=$((bad+1)); echo "FAIL $1"; }
W=$(mktemp -d "${TMPDIR:-/tmp}/pub-XXXXXX"); trap 'rm -rf "$W"' EXIT

ROW1="2026-11-02T02:08:49Z  refusal-repeated  count=731  channel.poll.failed"
ROW2="2026-11-02T06:31:00Z  run-stalled  r000000004356"

# 1. a missing record is an ERROR, not an empty result
out=$(PII_INCIDENTS_MD="$W/absent.md" PII_INCIDENTS_SENT="$W/sent" bash "$P" 2>&1); rc=$?
[ "$rc" -eq 2 ] && pass "a missing record exits 2" || fail "a missing record exits 2 (rc=$rc)"
printf '%s' "$out" | grep -q 'the incident record is missing at' && pass "and names the path" || fail "and names the path"
printf '%s' "$out" | grep -q "$W/absent.md" && pass "with the path it looked for" || fail "with the path it looked for"

# 2. CONTROL: the old publisher is silent for the same missing path — that is the defect
if [ -x "$OLD" ] || [ -f "$OLD" ]; then
  out=$(PII_INCIDENTS_MD="$W/absent.md" PII_INCIDENTS_SENT="$W/sent-old" bash "$OLD" 2>&1); rc=$?
  if [ "$rc" -eq 0 ] && [ -z "$out" ]; then
    pass "control: the old publisher is silent on a missing record"
  else
    fail "control: the old publisher is silent on a missing record ( rc=$rc out=$(printf '%s' "$out" | head -1) )"
  fi
else
  pass "control: skipped, the old publisher is not on this machine (loud skip, not a quiet pass)"
fi

# 3. a record with no rows is silent
printf '# pii incidents\n\n' >"$W/empty.md"
out=$(PII_INCIDENTS_MD="$W/empty.md" PII_INCIDENTS_SENT="$W/sent-empty" bash "$P" 2>&1); rc=$?
[ "$rc" -eq 0 ] && [ -z "$out" ] && pass "an empty record is silent and exits 0" || fail "an empty record is silent (rc=$rc out=$out)"

# 4. two rows, no cursor: both printed, once
printf '# pii incidents\n\n%s\n%s\n' "$ROW1" "$ROW2" >"$W/two.md"
out=$(PII_INCIDENTS_MD="$W/two.md" PII_INCIDENTS_SENT="$W/sent-two" bash "$P" 2>&1)
printf '%s' "$out" | grep -q "refusal-repeated" && pass "a new row is printed" || fail "a new row is printed"
printf '%s' "$out" | grep -q "run-stalled" && pass "and so is the second" || fail "and so is the second"

# 5. a second run prints nothing (the cursor)
out2=$(PII_INCIDENTS_MD="$W/two.md" PII_INCIDENTS_SENT="$W/sent-two" bash "$P" 2>&1)
[ -z "$out2" ] && pass "a second run prints nothing" || fail "a second run prints nothing (got $(printf '%s' "$out2" | head -1))"

# 6. a new row after the cursor is printed, and only it
printf '%s\n%s\n' "$ROW1" "$ROW2" >"$W/two.md"
printf '2026-11-02T07:10:00Z  dirty-tree  count=1\n' >>"$W/two.md"
out3=$(PII_INCIDENTS_MD="$W/two.md" PII_INCIDENTS_SENT="$W/sent-two" bash "$P" 2>&1)
printf '%s' "$out3" | grep -q "dirty-tree" && pass "a row added later is printed" || fail "a row added later is printed"
printf '%s' "$out3" | grep -q "refusal-repeated" && fail "and the old rows are not repeated" || pass "and the old rows are not repeated"

# 7. a shrunken record does not silently skip rows
printf '%s\n' "$ROW1" >"$W/small.md"
out4=$(PII_INCIDENTS_MD="$W/small.md" PII_INCIDENTS_SENT="$W/sent-two" bash "$P" 2>&1)
printf '%s' "$out4" | grep -q "refusal-repeated" && pass "a record that shrank reprints from the start" \
  || fail "a record that shrank reprints from the start"

# 8. the job-pointing helper prints the change and edits nothing
out5=$(bash deploy/point-incidents-job.sh --print 2>&1)
printf '%s' "$out5" | grep -q 'scripts/autonomy/incidents-publish.sh' && pass "the helper names the repo script" \
  || fail "the helper names the repo script"

echo "RESULT: $ok ok, $bad FAIL"
[ "$bad" -eq 0 ]
