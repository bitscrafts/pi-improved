#!/usr/bin/env bash
# ensure-write-dirs.sh DIR... — make every directory the gate writes to, so the disk floor judges room, not existence.
#
# disk-floor.sh reports a path that does not exist as MISSING, and the cycle reported that as a shortage of disk.
# Measured 2026-11-05: row 397's worker finished and its gate was skipped on a volume with 112 GB free, because
# _tmp/tools-gate/llvm-cov-target had been cleaned away and nothing made it again. The cycle owns these dirs; it makes
# them. A path that cannot be made (a file is in the way, a read-only mount) is NOT hidden: this exits 1 and names
# it, and disk-floor.sh will still say MISSING for it, so a real fault stays visible.
#
# The volume roots (/, /tmp, /projects) exist by definition and are skipped. It never deletes, moves or empties.
RC=0
for D in "$@"; do
  case "$D" in /|/tmp|/projects|"") continue ;; esac
  [ -d "$D" ] && continue
  if ! mkdir -p "$D" 2>/dev/null; then echo "ensure-write-dirs: cannot make $D" >&2; RC=1; fi
done
exit "$RC"
