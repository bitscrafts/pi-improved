#!/usr/bin/env bash
# stop-cycle.sh --yes — stop a running autonomy cycle WITHOUT the pattern-kill trap.
#
# WHY THIS FILE EXISTS (measured, three times in one day): `pgrep -f cycle.sh` / `pkill -f fresh-gate` match ANY
# command line containing that string -- including the shell that is running the kill. Each time the caller killed
# itself: exit 1, empty output, no result. The tv-mirror project's own wrapper documents the same failure ("the old
# pattern-matching version killed its own caller, exit 144").
#
# The rule this implements: never kill by a string you also carry. Match on the TARGET's own argv from /proc, and
# exclude the caller and its parent explicitly.
set -u

[ "${1:-}" = "--yes" ] || { echo "stop-cycle.sh: refusing without --yes (this ends a running cycle)"; exit 2; }

me=$$; parent=$PPID; found=0
for p in $(pgrep -f 'scripts/autonomy/cycle\.sh' || true); do
  [ "$p" = "$me" ] && continue
  [ "$p" = "$parent" ] && continue
  argv=$(tr '\0' ' ' < "/proc/$p/cmdline" 2>/dev/null) || continue
  case "$argv" in
    *"/bin/sh -c "*) continue ;;                                   # a wrapper shell, not the cycle itself
    *cycle.sh*) : ;;                                               # the cycle invocation
    *) continue ;;
  esac
  if kill "$p" 2>/dev/null; then echo "stopped cycle pid=$p argv=$argv"; found=1; fi
done
[ "$found" = 1 ] || echo "no cycle was running"
