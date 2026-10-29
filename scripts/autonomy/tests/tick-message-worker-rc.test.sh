#!/usr/bin/env bash
# tick-message-worker-rc.test.sh — the tick's phone line for a stopped worker says whether the loop will resume it (spec 313).
# Takes the case block out of scripts/autonomy/tick.sh at run time and runs it with a stub say_once that records
# the key and the lines. Never runs tick.sh, a cycle or a worker. Exits 0 only if every assertion holds.
set -u
cd "$(dirname "$0")/../../.." || exit 1
TICK=scripts/autonomy/tick.sh
OK=0
FAILS=0
# check <name> <got> <want>: an exact comparison, one ok or FAIL line per assertion
check(){
  if [ "$2" = "$3" ]; then echo "ok $1"; OK=$((OK+1)); else echo "FAIL $1"; echo "  got '$2', want '$3'" >&2; FAILS=$((FAILS+1)); fi
}
# the live case block: from the case line on OUT to its closing esac
BLOCK=$(sed -n '/^case "\$OUT" in/,/^esac/p' "$TICK")

# run <OUT> <ITEM> <NEXT>: runs the live block; prints KEY and LINE records. NEXT=<unset> leaves NEXT unset.
run(){
  (
    OUT=$1; ITEM=$2
    if [ "$3" = "<unset>" ]; then unset NEXT; else NEXT=$3; fi
    LASTLOG=_tmp/autonomy/logs/cycle-1.log; HEAD=abc1234
    say_once(){ local k=$1; shift; printf 'KEY %s\n' "$k"; printf 'LINE %s\n' "$@"; }
    eval "$BLOCK"
  )
}

# (0) the block is the live one, with every outcome name in it, so an empty extraction fails loudly
for w in esac done verify-failed gate-failed worker-rc- dirty-tree bad-plan window-closed; do
  check "the live block has the case for $w" "$(grep -cF -- "$w" <<<"$BLOCK" | grep -qv '^0$' && echo yes || echo no)" "yes"
done

A=$(run worker-rc-1 A1 $'A1\ttitle')
B=$(run worker-rc-1 A1 $'A2\ttitle')
C=$(run worker-rc-9 A1 NONE)

# (a) a row the queue will resume names itself as resumed
check "a-resumed-row-says-resumed" "$(grep -c 'resuming with its session' <<<"$A" | grep -q '^1$' && grep -q ' A1 ' <<<"$A" && echo yes || echo no)" "yes"
check "a-resumed-row-key" "$(grep '^KEY' <<<"$A")" "KEY resumed:A1:worker-rc-1"

# (b) a row with no session, or not offered, asks for a hand
check "a-row-with-no-session-asks-for-a-hand" "$(grep -c 'no session to resume' <<<"$B" | grep -q '^1$' && grep -q ' A1 ' <<<"$B" && echo yes || echo no)" "yes"
check "a-row-with-no-session-key" "$(grep '^KEY' <<<"$B")" "KEY bad:A1:worker-rc-1"

# (c) an empty queue asks for a hand
check "an-empty-queue-asks-for-a-hand" "$(grep -c 'no session to resume' <<<"$C" | grep -q '^1$' && echo yes || echo no)" "yes"
check "an-empty-queue-key" "$(grep '^KEY' <<<"$C")" "KEY bad:A1:worker-rc-9"

# (d) a tick whose pick failed (NEXT unset) still reports, and does not abort
check "a-tick-with-no-pick-still-reports" "$(grep -c 'no session to resume' <<<"$(run worker-rc-1 A1 '<unset>')" | grep -q '^1$' && echo yes || echo no)" "yes"

# (e) controls: failures keep today's sentence and key
V=$(run verify-failed A1 $'A1\ttitle')
check "CONTROL-a-verify-failure-is-not-a-resume" "$(cat <<<"$V")" "$(printf 'KEY bad:A1:verify-failed\nLINE pii autonomy · A1 needs a hand (verify-failed)\nLINE log: cycle-1.log')"
G=$(run gate-failed A1 $'A1\ttitle')
check "CONTROL-a-gate-failure-keeps-its-line" "$(grep '^LINE pii autonomy' <<<"$G")" "LINE pii autonomy · A1 needs a hand (gate-failed)"

# (f) the two worker-rc verdicts carry different keys
check "CONTROL-the-two-keys-differ" "$([ "$(grep '^KEY' <<<"$A")" != "$(grep '^KEY' <<<"$B")" ] && echo yes || echo no)" "yes"

echo "$OK ok, $FAILS FAIL"
[ "$FAILS" -eq 0 ]
