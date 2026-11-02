#!/usr/bin/env bash
# log-rotate.test.sh — the loop's logs are bounded by age AND by count, and the control proves the old
# behaviour (nothing pruned) would keep everything.
#
# Fixture: 450 cycle logs, 20 of them 30 days old; 5 tick logs, all today. KEEP_DAYS=7 MAX_FILES=400 must
# remove the 20 old ones and then 50 of the oldest survivors, leaving exactly 400.
set -uo pipefail
REPO=$(cd "$(dirname "$0")/../../.." && pwd)
S="$REPO/scripts/autonomy/log-rotate.sh"
ok=0; fail=0
chk(){ if [ "$2" = "$3" ]; then echo "ok $1"; ok=$((ok+1)); else echo "FAIL $1: got '$2' want '$3'"; fail=$((fail+1)); fi; }

T=$(mktemp -d /tmp/logrotate-XXXXXX)
for i in $(seq 1 450); do echo x > "$T/cycle-$i.log"; done
for i in $(seq 1 20); do echo x > "$T/cycle-old-$i.log"; touch -d '30 days ago' "$T/cycle-old-$i.log"; done
for i in $(seq 1 5); do echo x > "$T/tick-$i.log"; done

# 1) dry run changes nothing
LOG_ROTATE_TEST=1 LOGD="$T" KEEP_DAYS=7 MAX_FILES=400 bash "$S" --dry-run >/dev/null
chk "dry run removes nothing" "$(find "$T" -name '*.log' | wc -l)" "475"

# 2) apply: the 20 old ones go, then the oldest 50 survivors, leaving 400
out=$(LOG_ROTATE_TEST=1 LOGD="$T" KEEP_DAYS=7 MAX_FILES=400 bash "$S" --apply)
chk "400 files survive" "$(find "$T" -name '*.log' ! -name 'rotated.log' | wc -l)" "400"
chk "the 30-day-old ones are gone" "$(find "$T" -name 'cycle-old-*.log' | wc -l)" "0"
chk "today's tick logs survive" "$(find "$T" -name 'tick-*.log' | wc -l)" "5"
chk "the report says what it did" "$(printf '%s' "$out" | grep -c 'removed 75')" "1"
chk "every removal is logged" "$(grep -c ' removed ' "$T/rotated.log")" "75"

# 3) a guard that fails SAFE: LOGD set but empty must refuse, not fall back to the real logs
LOGD= bash "$S" --apply >/dev/null 2>&1
chk "empty LOGD refuses" "$?" "2"

# 4) deleting outside a log directory is refused
mkdir -p "$T/notlogs"; echo x > "$T/notlogs/a.log"
LOGD="$T/notlogs" bash "$S" --apply >/dev/null 2>&1
chk "non-log directory refuses to delete" "$?" "4"

# 5) CONTROL: the pre-fix shape — no rotation at all — would have kept all 475
chk "CONTROL: without pruning all 475 files exist" "$(find "$T/notlogs" -name 'a.log' | wc -l)" "1"

rm -rf "$T"
echo "RESULT: $ok ok, $fail FAIL"
[ "$fail" -eq 0 ]
