#!/usr/bin/env bash
# cycle-disk-wiring.test.sh — the cycle's disk floor receives every gate write dir as its own argument (spec 275).
# Reads the live call out of cycle.sh at run time and evaluates its argument text; never runs cycle.sh, a gate or a worker.
# Exits 0 only if every assertion holds. Scratch lives under _tmp/ and is removed by this script's own trap.
set -u
cd "$(dirname "$0")/../../.." || exit 1
CYCLE=scripts/autonomy/cycle.sh
ENV=scripts/autonomy/env.sh
OK=0
FAILS=0
# check <name> <got> <want>: an exact comparison, one ok or FAIL line per assertion
check(){
  if [ "$2" = "$3" ]; then echo "ok $1"; OK=$((OK+1)); else echo "FAIL $1"; echo "  got '$2', want '$3'" >&2; FAILS=$((FAILS+1)); fi
}
# calls <file>: the non-comment lines that name disk-floor.sh, as <lineno>:<text> (a comment's first non-blank character is #)
calls(){ grep -nF 'disk-floor.sh' "$1" | grep -vE '^[0-9]+:[[:blank:]]*#'; }
# first_line <file> <text>: the number of the first non-comment line that contains <text>
first_line(){ grep -nF -- "$2" "$1" | grep -vE '^[0-9]+:[[:blank:]]*#' | head -1 | cut -d: -f1; }
# args_of <file>: the text after disk-floor.sh up to the closing ) on the call line
args_of(){ local t; t=$(calls "$1" | head -1); t=${t#*:}; t=${t#*disk-floor.sh}; printf '%s' "${t%%)*}"; }
# has <want> <arg...>: 0 when <want> is one of the arguments, matched exactly
has(){ local w=$1 a; shift; for a in "$@"; do [ "$a" = "$w" ] && return 0; done; return 1; }
# facts <args>: evaluates the argument text with GATE_WRITE_DIRS set to the two planted dirs, one yes/no per property
facts(){
  (
    export GATE_WRITE_DIRS="$PD1 $PD2" EXTRA
    eval "set -- $1"
    FLOOR=no; case "${1:-}" in ''|*[!0-9]*) ;; *) FLOOR=yes ;; esac
    [ "$#" -gt 0 ] && shift
    PROJ=no; has /projects "$@" && PROJ=yes
    D1=no; has "$PD1" "$@" && D1=yes
    D2=no; has "$PD2" "$@" && D2=yes
    echo "floor=$FLOOR projects=$PROJ dir1=$D1 dir2=$D2"
  )
}
ALL="floor=yes projects=yes dir1=yes dir2=yes"
# wired <args>: wired or unwired, by whether every property of facts holds
wired(){ if [ "$(facts "$1")" = "$ALL" ]; then echo wired; else echo unwired; fi; }

T=_tmp/cycle-disk-wiring.$$
mkdir -p "$T"
trap 'find "$T" -depth -delete 2>/dev/null' EXIT
PD1=$(pwd)/$T/write-a; PD2=$(pwd)/$T/write-b; EXTRA=$(pwd)/$T/extra
mkdir -p "$PD1" "$PD2" "$EXTRA"

# (a) exactly one non-comment call site; a comment naming the helper above it does not change the arguments read
check "exactly one live line calls disk-floor.sh" "$(calls "$CYCLE" | wc -l | tr -d ' ')" "1"
NO=$(calls "$CYCLE" | head -1 | cut -d: -f1)
ARGS=$(args_of "$CYCLE")
FIX=$T/cycle-comment.sh
sed "${NO}i\\  # disk-floor.sh" "$CYCLE" >"$FIX"
check "a comment naming disk-floor.sh does not change the arguments read" "$(args_of "$FIX")" "$ARGS"

# (b) the arguments, evaluated: a numeric floor, /projects, and each planted write dir as its own argument
check "the live call hands the helper a floor, /projects and each write dir" "$(facts "$ARGS")" "$ALL"

# (c) the live list, sourced: GATE_WRITE_DIRS yields at least five paths after the floor and /projects
LIVE=$(unset GATE_WRITE_DIRS; . "$ENV"; set -- 20000 /projects $GATE_WRITE_DIRS; shift 2; echo $#)
check "env.sh lists at least five write dirs" "$([ "$LIVE" -ge 5 ] 2>/dev/null && echo yes || echo no)" "yes"

# (d) the floor guards the gate: the call comes after the GATE_ON=1 decision
GL=$(first_line "$CYCLE" 'GATE_ON=1')
check "the floor call is below the GATE_ON=1 decision" "$([ -n "$GL" ] && [ -n "$NO" ] && [ "$NO" -gt "$GL" ] && echo yes || echo no)" "yes"

# (e) CONTROL: the same assertions redden a planted unwiring (' $GATE_WRITE_DIRS' removed from a scratch copy of the text)
UNWIRED=${ARGS// \$GATE_WRITE_DIRS/}
check "the checker reddens a planted unwiring" "$(wired "$UNWIRED")" "unwired"

# (f) CONTROL: a correct call with one extra existing directory still passes
NEAR="$ARGS \"\$EXTRA\""
check "the near-miss stays green" "$(wired "$NEAR")" "wired"

# (g) CONTROL: a quoted expansion collapses the five paths into one argument, and is refused
QUOTED=${ARGS// \$GATE_WRITE_DIRS/ \"\$GATE_WRITE_DIRS\"}
check "CONTROL a quoted expansion is refused" "$(wired "$QUOTED")" "unwired"

echo "$OK ok, $FAILS FAIL"
[ "$FAILS" -eq 0 ]
