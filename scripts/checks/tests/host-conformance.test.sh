#!/usr/bin/env bash
# The host conformance probe, tested without a host — and with a control per check, because a probe that reports
# five passes when it could not look is worse than no probe: it certifies the machine.
set -uo pipefail
cd "$(dirname "$0")/../../.." || exit 1
P=scripts/checks/host-conformance.sh
FX=scripts/checks/fixtures
ok=0; bad=0
pass(){ ok=$((ok+1)); echo "ok   $1"; }
fail(){ bad=$((bad+1)); echo "FAIL $1"; }
W=$(mktemp -d "${TMPDIR:-/tmp}/hc-XXXXXX"); trap 'rm -rf "$W"' EXIT

# 1. today's host shape: five defects, five FAILs
out=$(bash "$P" --fixture "$FX/host-red.json" 2>&1); rc=$?
[ "$rc" -eq 1 ] && pass "the red fixture exits 1" || fail "the red fixture exits 1 (rc=$rc)"
printf '%s' "$out" | grep -q 'HOST-CONFORMANCE: 0 passed, 5 failed' && pass "and reports 0 passed, 5 failed" \
  || fail "and reports 0 passed, 5 failed ($(printf '%s' "$out" | tail -1))"

# 2. a conformant host: five PASSes
out=$(bash "$P" --fixture "$FX/host-green.json" 2>&1); rc=$?
[ "$rc" -eq 0 ] && pass "the green fixture exits 0" || fail "the green fixture exits 0 (rc=$rc)"
printf '%s' "$out" | grep -q 'HOST-CONFORMANCE: 5 passed, 0 failed' && pass "and reports 5 passed" \
  || fail "and reports 5 passed"

# 3. one defect, one red — and it is the right one
out=$(bash "$P" --fixture "$FX/host-one-defect.json" 2>&1)
printf '%s' "$out" | grep -q 'HOST-CONFORMANCE: 4 passed, 1 failed' && pass "one defect turns exactly one check red" \
  || fail "one defect turns exactly one check red"
printf '%s' "$out" | grep -q '^FAIL  durable stores' && pass "and it is the store check" || fail "and it is the store check"

# 4. a control per check: apply one defect to the green fixture and require exactly that check to fail
mk_defect(){
  python3 - "$FX/host-green.json" "$W/$1.json" "$1" <<'PY'
import json,sys
src,dst,which=sys.argv[1],sys.argv[2],sys.argv[3]
d=json.load(open(src))
if which=="config":       d["config"]='[core]\ntrajectory_dir = "/root/.local/state/pii/trajectory"\n'
if which=="stores":       d["runtime_ls"]="/run/user/1000/pii/channel.redb"
if which=="health":       d["health_plain"]={"rc":2,"stderr":"no daemon answered"}
if which=="failures":     d["log"]=[{"kind":"daemon.started","ts":"t"},{"kind":"channel.post.failed","level":"error","ts":"t2"}]
if which=="alarm":        d["incidents"]="daemon-start-not-ready start ts=t time_to_notice_ms=144"
json.dump(d,open(dst,"w"))
PY
}
declare -A EXPECT=([config]="config paths" [stores]="durable stores" [health]="health from a plain shell" \
                   [failures]="no failures since the last start" [alarm]="no false start alarm")
for which in config stores health failures alarm; do
  mk_defect "$which"
  out=$(bash "$P" --fixture "$W/$which.json" 2>&1)
  n=$(printf '%s' "$out" | grep -c '^FAIL')
  if [ "$n" -eq 1 ] && printf '%s' "$out" | grep -q "^FAIL  ${EXPECT[$which]}"; then
    pass "control: only the $which defect reddens only its own check"
  else
    fail "control: only the $which defect reddens only its own check (got $n FAIL)"
  fi
done

# 5. an unreachable host REFUSES, and does not pass
out=$(PII_HOST=10.255.255.1 PII_SSH_TIMEOUT=2 bash "$P" 2>&1); rc=$?
[ "$rc" -eq 2 ] && pass "an unreachable host refuses with rc 2" || fail "an unreachable host refuses with rc 2 (rc=$rc)"
printf '%s' "$out" | grep -q 'REFUSED' && pass "and says REFUSED" || fail "and says REFUSED"

# 6. no host and no fixture refuses rather than guessing
out=$(env -u PII_HOST bash "$P" 2>&1); rc=$?
[ "$rc" -eq 2 ] && pass "no host refuses with rc 2" || fail "no host refuses with rc 2 (rc=$rc)"

# 7. the gate row skips loudly when there is no host, and a skip is not a pass
out=$(env -u PII_HOST bash scripts/checks/gates.sh --gate M5 2>&1)
printf '%s' "$out" | grep -qE 'SKIP +M5' && pass "the gate prints SKIP for the host row" || fail "the gate prints SKIP for the host row"
printf '%s' "$out" | grep -qE 'SKIP +M5 +host conformance' && pass "and the skip names the row" || fail "and the skip names the row"

echo "RESULT: $ok ok, $bad FAIL"
[ "$bad" -eq 0 ]
