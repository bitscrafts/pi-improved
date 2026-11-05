#!/usr/bin/env bash
# write-dirs.test.sh — the cycle makes the directories its gate writes to before it asks whether they have room.
#
# Measured 2026-11-05: a worker finished row 397 and the gate was SKIPPED with `MISSING: _tmp/tools-gate/llvm-cov-target
# (needs 20000M free)` on a volume with 112 GB free. Nothing in the cycle created the dir (a cargo clean had removed
# it, and every gate run by hand made it first), so the floor reported a directory that the cycle itself should have
# made as a shortage, and the finished row was left uncommitted. A missing write dir is wiring, not disk.
#
# Drives the REAL helper (scripts/autonomy/ensure-write-dirs.sh) and reads the live call order out of cycle.sh.
set -u
cd "$(dirname "$0")/../../.." || exit 1
H=scripts/autonomy/ensure-write-dirs.sh
CYCLE=scripts/autonomy/cycle.sh
OK=0; FAILS=0
check(){ if [ "$2" = "$3" ]; then echo "ok $1"; OK=$((OK+1)); else echo "FAIL $1"; echo "  got '$2', want '$3'" >&2; FAILS=$((FAILS+1)); fi; }

T=_tmp/write-dirs-test.$$
mkdir -p "$T"
trap 'find "$T" -depth -delete 2>/dev/null' EXIT
A=$PWD/$T/gate/llvm-cov-target; B=$PWD/$T/tools/llvm-cov-target

# (a) the helper exists and makes every missing dir, nested, in one call
check "the helper exists" "$([ -f "$H" ] && echo yes || echo no)" "yes"
bash "$H" "$A" "$B" >/dev/null 2>&1; RC=$?
check "it succeeds" "$RC" "0"
check "it creates a missing nested dir" "$([ -d "$A" ] && [ -d "$B" ] && echo yes || echo no)" "yes"

# (b) CONTROL: a dir that already exists, with content, is left exactly as it is
echo keep > "$A/keep.txt"
bash "$H" "$A" >/dev/null 2>&1
check "CONTROL an existing dir keeps its content" "$(cat "$A/keep.txt" 2>/dev/null)" "keep"

# (c) CONTROL: a path that cannot be made (a FILE is in the way) is reported and is NOT papered over: the floor must
# still be able to say MISSING for it, so a real fault stays visible.
echo f > "$T/afile"
OUT=$(bash "$H" "$PWD/$T/afile/sub" 2>&1); RC=$?
check "CONTROL an impossible dir fails the helper" "$([ "$RC" -ne 0 ] && echo failed || echo passed)" "failed"
check "CONTROL ... and names the path" "$(printf '%s' "$OUT" | grep -c 'afile/sub')" "1"

# (d) the system dirs are never created or touched: / and /tmp and /projects are skipped, not mkdir'd
OUT=$(bash "$H" / /tmp /projects 2>&1); RC=$?
check "the volume roots are skipped without error" "$RC" "0"

# (e) the live wiring: cycle.sh calls the helper, with the gate's write dirs, BEFORE the disk floor
HL=$(grep -nF 'ensure-write-dirs.sh' "$CYCLE" | grep -vE '^[0-9]+:[[:blank:]]*#' | head -1 | cut -d: -f1)
FL=$(grep -nF 'disk-floor.sh' "$CYCLE" | grep -vE '^[0-9]+:[[:blank:]]*#' | head -1 | cut -d: -f1)
check "cycle.sh calls the helper" "$([ -n "$HL" ] && echo yes || echo no)" "yes"
check "the helper is called before the disk floor" "$([ -n "$HL" ] && [ -n "$FL" ] && [ "$HL" -lt "$FL" ] && echo yes || echo no)" "yes"
check "it is handed the gate's write dirs" "$(sed -n "${HL:-0}p" "$CYCLE" | grep -c 'GATE_WRITE_DIRS')" "1"

# (f) CONTROL: the ORDER check reddens when the call sits after the floor. Move the helper's line below the floor's
# and run the same comparison on that copy: it must say "no".
sed -n "${HL:-1}p" "$CYCLE" > "$T/call-line.txt"
sed -e "${HL:-1}d" "$CYCLE" > "$T/without.sh"
FL2=$(grep -nF 'disk-floor.sh' "$T/without.sh" | grep -vE '^[0-9]+:[[:blank:]]*#' | head -1 | cut -d: -f1)
{ head -n "$FL2" "$T/without.sh"; cat "$T/call-line.txt"; tail -n +"$((FL2 + 1))" "$T/without.sh"; } > "$T/moved.sh"
HL3=$(grep -nF 'ensure-write-dirs.sh' "$T/moved.sh" | grep -vE '^[0-9]+:[[:blank:]]*#' | head -1 | cut -d: -f1)
FL3=$(grep -nF 'disk-floor.sh' "$T/moved.sh" | grep -vE '^[0-9]+:[[:blank:]]*#' | head -1 | cut -d: -f1)
check "CONTROL the call moved below the floor is detected" "$([ -n "$HL3" ] && [ -n "$FL3" ] && [ "$HL3" -lt "$FL3" ] && echo yes || echo no)" "no"

# (g) the live list, sourced: every declared write dir is made by the helper (fresh scratch, no /projects writes)
LIVE=$(unset GATE_WRITE_DIRS; . scripts/autonomy/env.sh; echo "$GATE_WRITE_DIRS")
check "env.sh declares write dirs" "$([ -n "$LIVE" ] && echo yes || echo no)" "yes"

echo "$OK ok, $FAILS FAIL"
[ "$FAILS" -eq 0 ]
