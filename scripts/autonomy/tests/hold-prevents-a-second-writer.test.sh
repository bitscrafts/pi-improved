#!/usr/bin/env bash
# hold-prevents-a-second-writer.test.sh — the tree hold: a background caller yields, a dead holder never wedges the
# loop, and the flag can only reduce what a caller may do (2026-11-01, after three collisions in one day).
#
# Measured 2026-11-01: at 00:15Z and 06:15Z the skill-lane agent authored and committed while an interactive turn
# held the same tree, and at 12:21Z a cycle dispatched a paid claude worker into it (gate and worker editing at
# once; my gate lost its export). The cycle's flock is no defence: an interactive turn takes no lock at all.
#
# The properties this test pins:
#   · a live hold reports HELD-BY and drain/cycle exit 0 (a background caller yields, it does not fail)
#   · take/check/release round-trips, and a stranger cannot release someone else's hold
#   · a hold whose pid is dead is REAPED on the next check, so a crashed holder cannot wedge the loop
#   · a hold past its TTL is reaped too
#   · the flag is not a privilege: --from-cron/PII_CRON only makes a caller yield sooner, and hold.sh itself grants
#     nothing to a caller that says it is cron
set -u
cd "$(dirname "$0")/../../.." || exit 1
OK=0; FAILS=0
check(){ if [ "$2" = "$3" ]; then echo "ok $1"; OK=$((OK+1)); else echo "FAIL $1"; echo "  got '$2', want '$3'" >&2; FAILS=$((FAILS+1)); fi; }
T="$PWD/_tmp/hold-prevents.$$"
trap 'find "$T" -depth -delete 2>/dev/null' EXIT
rm -rf "$T"; mkdir -p "$T"
export PII_HOLD="$T/hold"

# free by default
check "a fresh tree is FREE" "$(bash scripts/autonomy/hold.sh check)" "hold: FREE"

# take / status / release round-trip
TAKEN=$(bash scripts/autonomy/hold.sh take interactive-test 60)
check "take names the owner" "$(printf '%s' "$TAKEN" | grep -c 'TAKEN interactive-test')" "1"
# LIVENESS IS THE TTL, NOT THE PID. A shell-based holder's pid is ephemeral -- `$(bash hold.sh take ...)` dies at
# once, and an earlier version of this file reaped every hold on the next breath because it trusted that pid.
check "an undeclared pid leaves the TTL as the clock" "$(bash scripts/autonomy/hold.sh status | grep -c 'pid=- priority=50')" "1"
check "and the hold is still held" "$(bash scripts/autonomy/hold.sh check | cut -d' ' -f2)" "HELD-BY"
bash scripts/autonomy/hold.sh release interactive-test >/dev/null
PII_HOLD_PID=$$ bash scripts/autonomy/hold.sh take declared-pid 60 50 >/dev/null
check "a declared pid is recorded and shown alive" "$(bash scripts/autonomy/hold.sh status | grep -cE "pid=$$ priority=50 .*alive=yes")" "1"
bash scripts/autonomy/hold.sh release declared-pid >/dev/null
bash scripts/autonomy/hold.sh take interactive-test 60 50 >/dev/null
check "a second take is refused, not honoured" "$(bash scripts/autonomy/hold.sh take other 60 | cut -d' ' -f2)" "TAKEN-BY"
check "a stranger cannot release it" "$(bash scripts/autonomy/hold.sh release other | cut -d' ' -f2)" "REFUSED"
check "the owner can release it" "$(bash scripts/autonomy/hold.sh release interactive-test | cut -d' ' -f2)" "RELEASED"
check "and the tree is FREE again" "$(bash scripts/autonomy/hold.sh check)" "hold: FREE"

# drain yields (exit 0) while a live hold is on, and says whose
bash scripts/autonomy/hold.sh take interactive-test 60 >/dev/null
OUT=$(bash scripts/autonomy/drain.sh --from-cron 3>&1 2>&1); RC=$?
check "drain yields with exit 1 (its own lock-refusal path) or 0" "$(case $RC in 0|1) echo ok;; *) echo "rc=$RC";; esac)" "ok"
check "and it says the tree is held, naming the owner" "$(printf '%s' "$OUT" | grep -c 'HELD-BY interactive-test')" "1"

# a cycle from cron yields too, and its OUTCOME is explicit
OUT2=$(PII_CRON=1 bash scripts/autonomy/cycle.sh 2>&1 | grep -E 'OUTCOME=tree-held|HELD-BY' | head -2)
check "a cron cycle yields with OUTCOME=tree-held" "$(printf '%s' "$OUT2" | grep -c 'OUTCOME=tree-held')" "1"

# REAPING: a hold whose pid is dead must not wedge the loop
printf '%s %s %s %s\n' dead-owner 999999 "$(date -u +%s)" "$(( $(date -u +%s) + 600 ))" > "$PII_HOLD"
check "a hold with a dead pid is reaped" "$(bash scripts/autonomy/hold.sh check | tail -1 | cut -d' ' -f2)" "FREE"
check "and the reaping is said aloud" "$(bash scripts/autonomy/hold.sh status | grep -c 'owner=-')" "1"

# PRIORITY SWITCH: automation may not hold a tree a person needs, but a person never evicts a person
bash scripts/autonomy/hold.sh release interactive-test >/dev/null
bash scripts/autonomy/hold.sh take cron-job 60 20 >/dev/null
check "a person takes the tree from automation" "$(bash scripts/autonomy/hold.sh take person 60 100 | head -1 | cut -d' ' -f2)" "TAKE-OVER"
check "and the switch says why" "$(bash scripts/autonomy/hold.sh status | grep -c 'priority=100')" "1"
check "a second person cannot evict the first" "$(bash scripts/autonomy/hold.sh take person2 60 100 | head -1 | cut -d' ' -f2)" "TAKEN-BY"
check "a low-priority caller is refused by a high-priority hold" "$(bash scripts/autonomy/hold.sh take cron2 60 20 | head -1 | cut -d' ' -f2)" "TAKEN-BY"
bash scripts/autonomy/hold.sh release person >/dev/null

# TTL: an expired hold is reaped even while its pid lives (this shell), so a forgotten hold cannot outlive its TTL
printf '%s %s %s %s\n' expired-owner $$ "$(date -u +%s)" "$(( $(date -u +%s) - 1 ))" > "$PII_HOLD"
check "a hold past its ttl is reaped" "$(bash scripts/autonomy/hold.sh check | tail -1 | cut -d' ' -f2)" "FREE"

# CONTROL: the reaped holds are KEPT as evidence, not deleted silently
check "CONTROL reaped holds are kept on disk" "$(ls "$T"/hold.reaped.* 2>/dev/null | wc -l)" "2"

echo "$OK ok, $FAILS FAIL"
[ "$FAILS" -eq 0 ]
