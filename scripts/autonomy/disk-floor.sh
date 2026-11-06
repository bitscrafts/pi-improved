#!/usr/bin/env bash
# disk-floor.sh FLOOR_MB PATH... — every PATH must have at least FLOOR_MB free on the volume it really writes to (spec 273).
# A symlink is followed (readlink -f) to the volume that holds the data, so a link into another volume is judged there.
# Exit 0 when every path passes, 1 when one is LOW, 4 when one is MISSING (wiring, not a shortage), 2 on bad usage. Reads df only: it never deletes, moves or writes a file.
case "${1:-}" in ""|*[!0-9]*) echo "usage: disk-floor.sh FLOOR_MB PATH..." >&2; exit 2 ;; esac
[ "$#" -ge 2 ] || { echo "usage: disk-floor.sh FLOOR_MB PATH..." >&2; exit 2; }
FLOOR=$1; shift
RC=0; MISS=0
for P in "$@"; do
  if [ ! -e "$P" ]; then echo "MISSING: $P"; MISS=1; continue; fi
  R=$(readlink -f "$P")
  read -r FREE TARGET < <(df -m --output=avail,target "$R" | tail -1)
  echo "$P -> $R free=${FREE}M mount=$TARGET"
  if [ "${FREE:-0}" -lt "$FLOOR" ]; then
    echo "LOW: $R free=${FREE}M floor=${FLOOR}M mount=$TARGET"; RC=1
  fi
done
[ "$MISS" -eq 1 ] && exit 4
exit "$RC"
