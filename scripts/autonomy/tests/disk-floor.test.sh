#!/usr/bin/env bash
# disk-floor.test.sh — the gate's disk floor checks every volume the gate writes to (spec 273).
# Exits 0 only if every assertion holds. Makes a scratch dir and links under _tmp/ only; never runs cycle.sh, the queue or a gate.
set -u
cd "$(dirname "$0")/../../.." || exit 1
H="$(pwd)/scripts/autonomy/disk-floor.sh"
CYCLE=scripts/autonomy/cycle.sh
ENV=scripts/autonomy/env.sh
OK=0
FAILS=0
# check <name> <got> <want>: an exact comparison, one ok or FAIL line per assertion
check(){
  if [ "$2" = "$3" ]; then echo "PASS $1"; OK=$((OK+1)); else echo "FAIL $1"; echo "  got '$2', want '$3'" >&2; FAILS=$((FAILS+1)); fi
}
# mount_of <path>: the mount point df reports for a path
mount_of(){ df -m --output=target "$1" | tail -1 | sed 's/^ *//;s/ *$//'; }
# run <args>: the helper's output, then rc=<its exit code> on the last line
run(){ bash "$H" "$@"; echo "rc=$?"; }

T=_tmp/disk-floor.$$
mkdir -p "$T/dir"
RD=$(readlink -f "$T/dir")
trap 'find "$T" -depth -delete 2>/dev/null' EXIT

# (a) a real directory with a floor of 1 passes and prints free=
OUT=$(run 1 "$T/dir")
check "a real directory above the floor passes" "$(echo "$OUT" | tail -1)" "rc=0"
check "it prints its free MB" "$(echo "$OUT" | grep -c "^$T/dir -> $RD free=[0-9]*M mount=")" "1"

# (b) the same directory with an impossible floor fails and names the resolved path in LOW:
OUT=$(run 99999999999 "$T/dir")
check "a directory below the floor fails" "$(echo "$OUT" | tail -1)" "rc=1"
check "it names the resolved path in LOW:" "$(echo "$OUT" | grep -c "^LOW: $RD free=")" "1"

# (c) a symlink to a directory on ANOTHER volume reports the TARGET's mount, not the link's
TM=$(mount_of /tmp); LM=$(mount_of "$T")
if [ "$TM" = "$LM" ]; then
  echo "SKIP cross-volume symlink: /tmp and $T are both on '$LM', so the mounts cannot differ here"
else
  ln -s /tmp "$T/tmplink"
  OUT=$(run 1 "$T/tmplink")
  check "a symlink resolves to its target and reports the target's mount" "$(echo "$OUT" | grep -c "^$T/tmplink -> /tmp free=[0-9]*M mount=$TM\$")" "1"
fi

# (d) a missing path exits 1 and is reported MISSING
OUT=$(run 1 "$T/nope")
check "a missing path exits 1" "$(echo "$OUT" | tail -1)" "rc=1"
check "a missing path is reported MISSING" "$(echo "$OUT" | grep -c "MISSING.*$T/nope")" "1"

# (e) two paths, one failing (missing): exit 1, the real directory is not LOW and the missing one is MISSING
OUT=$(run 1 "$T/dir" "$T/nope")
check "one failing path exits 1" "$(echo "$OUT" | tail -1)" "rc=1"
check "the passing directory is not named LOW:" "$(echo "$OUT" | grep -c "LOW:.*$T/dir")" "0"
check "the failing path is named MISSING" "$(echo "$OUT" | grep -c "MISSING.*$T/nope")" "1"

# (f) no arguments and a non-numeric floor both exit 2
check "no arguments exits 2" "$(run | tail -1)" "rc=2"
check "a non-numeric floor exits 2" "$(run abc "$T/dir" | tail -1)" "rc=2"

# (g) CONTROL: the helper deletes, moves and writes nothing: no rm, no mv, no file redirect in its code
PAT='(^|[;&|(])[[:space:]]*(rm|mv)[[:space:]]|[^-]>+[[:space:]]*[^[:space:]&]'
check "CONTROL the helper has no rm, mv or file redirect" "$(grep -cE "$PAT" "$H" 2>/dev/null)" "0"

# (h) cycle.sh calls the helper and no longer reads /projects alone; env.sh names the write dirs once
check "cycle.sh calls disk-floor.sh" "$(grep -q 'disk-floor.sh' "$CYCLE" && echo yes || echo no)" "yes"
check "cycle.sh no longer measures /projects alone" "$(grep -qF 'df -m --output=avail /projects' "$CYCLE" && echo yes || echo no)" "no"
check "env.sh defines GATE_WRITE_DIRS" "$(grep -q 'GATE_WRITE_DIRS' "$ENV" && echo yes || echo no)" "yes"
check "the three scripts pass bash -n" "$(bash -n "$H" && bash -n "$CYCLE" && bash -n "$ENV" && echo yes || echo no)" "yes"

echo "$OK PASS, $FAILS FAIL"
[ "$FAILS" -eq 0 ]
