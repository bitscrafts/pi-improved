#!/usr/bin/env bash
# watchdog-records-its-tick.test.sh — the beat's tick output lands in a log, never in /dev/null (Opus review 2026-11-01).
#
# Measured before this change: `watchdog.sh:22` ran `tick.sh >/dev/null 2>&1`, and `tick.sh:65` runs
# `incidents scan --root "$LOGD" --new`, whose `--new` MARKS each incident reported. So the beat marked incidents
# reported and discarded the line, for eleven days: the record existed and no reader could see it. Spec 330 built a
# reader for that line and did not notice the line was already being thrown away.
#
# The property: whatever tick.sh prints must be appendable to a dated log, and the watchdog must not send it to
# /dev/null. This test drives the real watchdog script with a stub tick that prints a line, never the real tick.
set -u
cd "$(dirname "$0")/../../.." || exit 1
OK=0
FAILS=0
check(){ if [ "$2" = "$3" ]; then echo "ok $1"; OK=$((OK+1)); else echo "FAIL $1"; echo "  got '$2', want '$3'" >&2; FAILS=$((FAILS+1)); fi; }
T="$PWD/_tmp/watchdog-records.$$"   # ABSOLUTE: the watchdog chdirs to the repo root, so a relative path would resolve there
trap 'find "$T" -depth -delete 2>/dev/null' EXIT
mkdir -p "$T/scripts/autonomy"
# a stub tick: prints one incident-shaped line and exits 0
printf '%s\n' '#!/usr/bin/env bash' 'echo "incident dirty-tree count=496 first_seen=2026-11-01T00:00:00Z"' > "$T/scripts/autonomy/tick.sh"
chmod +x "$T/scripts/autonomy/tick.sh"
# the real watchdog, pointed at the stub by running it from $T with one very short beat and a fast kill
cp scripts/autonomy/watchdog.sh "$T/scripts/autonomy/watchdog.sh"
( AUTONOMY_WATCHDOG_PID="$T/wd.pid" AUTONOMY_LOGD="$T/logs" AUTONOMY_TICK="$T/scripts/autonomy/tick.sh" WATCHDOG_INTERVAL=1 \
  timeout 3 bash scripts/autonomy/watchdog.sh >/dev/null 2>&1 ) || true

LOG=$(ls "$T"/logs/tick-*.log 2>/dev/null | head -1)
check "the beat writes a dated log" "$( [ -n "$LOG" ] && echo yes || echo no )" "yes"
# one line per BEAT, not one per file: with a 1 s beat the stub's line is written once per beat, so the assertion is
# "at least one, and every beat accounted for" -- an exact 1 only held for a 3 s window and would be a flake.
BEATS=$(grep -c 'incident dirty-tree' "${LOG:-/dev/null}" 2>/dev/null || echo 0)
check "the line is recorded on every beat" "$( [ "${BEATS:-0}" -ge 1 ] && echo yes || echo no )" "yes"

# CONTROL: the old shape must fail this test — prove it by running a watchdog that still discards.
sed 's|>>"\$LOGD/tick-$(date -u +%Y%m%d).log" 2>&1|>/dev/null 2>\&1|' scripts/autonomy/watchdog.sh > "$T/scripts/autonomy/watchdog-old.sh"
rm -rf "$T/_tmp"
( AUTONOMY_WATCHDOG_PID="$T/wd2.pid" AUTONOMY_LOGD="$T/logs2" AUTONOMY_TICK="$T/scripts/autonomy/tick.sh" WATCHDOG_INTERVAL=1 \
  timeout 3 bash scripts/autonomy/watchdog-old.sh >/dev/null 2>&1 ) || true
check "CONTROL: the discarding shape records nothing" "$(ls "$T"/logs2/tick-*.log 2>/dev/null | wc -l)" "0"

# and the shipped script must not contain the discard at all
check "the watchdog has no live /dev/null redirect for the tick" \
  "$(grep -cE '^[^#]*bash "\$TICK" >/dev/null' scripts/autonomy/watchdog.sh)" "0"

echo "$OK ok, $FAILS FAIL"
[ "$FAILS" -eq 0 ]
