#!/usr/bin/env bash
# detection-before-delivery.test.sh — a delivery fault must never suppress detection (2026-11-01).
#
# Measured before this change: the tick's incident scan sat AFTER the gateway block, and that block ends with
# `exit 0` on the belief that the gateway is down. Its own log said "the gateway is down and would not start —
# messages cannot be delivered" while the gateway had been up for days (its CLI probe costs 10s per call here and
# misfires under load), so the beat ended there and NO incident was ever scanned or published. Detection is the
# product; delivery is a surface. This test drives the real tick with a gateway that is DOWN and asserts the
# incident line still comes out.
set -u
cd "$(dirname "$0")/../../.." || exit 1
OK=0; FAILS=0
check(){ if [ "$2" = "$3" ]; then echo "ok $1"; OK=$((OK+1)); else echo "FAIL $1"; echo "  got '$2', want '$3'" >&2; FAILS=$((FAILS+1)); fi; }
T="$PWD/_tmp/detection-before.$$"
trap 'find "$T" -depth -delete 2>/dev/null' EXIT
rm -rf "$T"; mkdir -p "$T/bin" "$T/logs"

# a stub pii that reports one incident, in the exact shape tick.sh parses
printf '%s\n' '#!/usr/bin/env bash' \
  'echo "x dirty-tree count=496 first_seen=2026-10-15T20:01:00Z (see x)"' > "$T/bin/pii-stub"
chmod +x "$T/bin/pii-stub"
# a hermes that always fails, and a pgrep that finds nothing: the gateway is "down" as far as the tick can tell
printf '%s\n' '#!/usr/bin/env bash' 'exit 1' > "$T/bin/hermes"; chmod +x "$T/bin/hermes"
printf '%s\n' '#!/usr/bin/env bash' 'exit 1' > "$T/bin/pgrep";  chmod +x "$T/bin/pgrep"

OUT=$(PATH="$T/bin:$PATH" PII_BIN="$T/bin/pii-stub" PII_REPORTED="$T/last-reported" \
      timeout 90 bash scripts/autonomy/tick.sh 2>&1) || true

check "the incident line is published" "$(printf '%s' "$OUT" | grep -c 'INCIDENT dirty-tree')" "1"
check "and it names the count" "$(printf '%s' "$OUT" | grep -c '496 times')" "1"
# CONTROL: the gateway block did fire (so the incident block cannot have been reached by the gateway being up)
check "CONTROL the gateway block also ran and complained" "$(printf '%s' "$OUT" | grep -c 'gateway is down')" "1"
# and the incident line comes BEFORE that complaint -- the ordering is the whole point
FIRST_INC=$(printf '%s\n' "$OUT" | grep -n 'INCIDENT dirty-tree' | head -1 | cut -d: -f1)
FIRST_GW=$(printf '%s\n' "$OUT" | grep -n 'gateway is down' | head -1 | cut -d: -f1)
check "detection precedes delivery" "$( [ -n "$FIRST_INC" ] && [ -n "$FIRST_GW" ] && [ "$FIRST_INC" -lt "$FIRST_GW" ] && echo yes || echo no )" "yes"

echo "$OK ok, $FAILS FAIL"
[ "$FAILS" -eq 0 ]
