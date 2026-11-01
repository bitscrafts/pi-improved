#!/usr/bin/env bash
# incidents-md-publishes-once.test.sh — the file surface for incidents: published once, never twice (2026-11-01).
set -u
cd "$(dirname "$0")/../../.." || exit 1
OK=0; FAILS=0
check(){ if [ "$2" = "$3" ]; then echo "ok $1"; OK=$((OK+1)); else echo "FAIL $1"; echo "  got '$2', want '$3'" >&2; FAILS=$((FAILS+1)); fi; }
T="$PWD/_tmp/incidents-md.$$"; trap 'find "$T" -depth -delete 2>/dev/null' EXIT
rm -rf "$T"; mkdir -p "$T/logs"
export AUTONOMY_LOGD="$T/logs" PII_INCIDENTS_MD="$T/INCIDENTS.md" PII_INCIDENTS_CURSOR="$T/INCIDENTS.cursor"

printf '%s\n' 'pii autonomy · INCIDENT dirty-tree — 496 times since 2026-10-15T20:01:00Z (see x/incidents.jsonl)' \
              'noise that is not an incident' > "$T/logs/tick-20261101.log"
bash scripts/autonomy/incidents-md.sh >/dev/null
check "it publishes the incident line" "$(grep -c 'dirty-tree' "$T/INCIDENTS.md")" "1"
check "and only incident lines" "$(grep -c 'noise that is not an incident' "$T/INCIDENTS.md" 2>/dev/null; true)" "0"
check "the row is dated and greppable" "$(grep -c 'T.* | dirty-tree | ' "$T/INCIDENTS.md")" "1"

OUT2=$(bash scripts/autonomy/incidents-md.sh)
check "a second run publishes nothing new" "$(grep -c 'dirty-tree' "$T/INCIDENTS.md")" "1"
check "and says it is up to date" "$(printf '%s' "$OUT2" | grep -c 'up to date')" "1"
check "--check agrees there is nothing pending" "$(bash scripts/autonomy/incidents-md.sh --check >/dev/null 2>&1; echo rc=$?)" "rc=0"

printf '%s\n' 'pii autonomy · INCIDENT verify-failed — 3 times since 2026-11-01T12:00:00Z (see x/incidents.jsonl)' >> "$T/logs/tick-20261101.log"
check "--check sees the pending one" "$(bash scripts/autonomy/incidents-md.sh --check >/dev/null 2>&1; echo rc=$?)" "rc=3"
bash scripts/autonomy/incidents-md.sh >/dev/null
check "and the new one is published" "$(grep -c 'verify-failed' "$T/INCIDENTS.md")" "1"
check "the old one is not repeated" "$(grep -c 'dirty-tree' "$T/INCIDENTS.md")" "1"

echo "$OK ok, $FAILS FAIL"
[ "$FAILS" -eq 0 ]
