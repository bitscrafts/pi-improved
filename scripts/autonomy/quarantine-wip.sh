#!/usr/bin/env bash
# quarantine-wip.sh <row> [reason] — put a parked row's working-tree changes aside so the queue can move on.
#
# WHY THIS EXISTS (measured 2026-11-03): a park left nine changed/untracked files in the tree, and the dispatcher
# refuses to dispatch on a dirty tree, so one parked row stopped every later row for the rest of the night. The
# parked work was GOOD (spec 352: five tests green) and had to survive, so this never deletes anything:
#   - every changed path is COPIED to /projects/_trash/<row>-<reason>-<ts>/ with the tree layout preserved
#   - a modified tracked file is then restored to its HEAD content with `git show HEAD:<path>` (NOT `git checkout --`,
#     which is banned in this workspace because it destroys uncommitted work)
#   - an untracked file is MOVED out of the tree (mv, never rm)
# Prints the aside directory on stdout. Exit 0 with "CLEAN" when there was nothing to do.
set -uo pipefail

N="${1:?usage: quarantine-wip.sh <row> [reason]}"
WHY="${2:-parked}"
ROOT="$(git rev-parse --show-toplevel 2>/dev/null)" || { echo "not in a git tree" >&2; exit 2; }
cd "$ROOT" || exit 2

dirty=$(git status --porcelain | wc -l)
if [ "$dirty" -eq 0 ]; then echo "CLEAN"; exit 0; fi

ts=$(date -u +%Y%m%dT%H%M%SZ)
ASIDE_ROOT="${QUARANTINE_ROOT:-/projects/_trash}"
aside="${ASIDE_ROOT}/${N}-${WHY}-${ts}"
mkdir -p "$aside" || exit 2
git status --porcelain > "$aside/status.txt"

kept=0
while IFS= read -r line; do
  [ -z "$line" ] && continue
  st="${line:0:2}"
  p="${line:3}"
  case "$p" in *" -> "*) p="${p##* -> }" ;; esac
  # never touch our own aside directory (it is normally outside the tree, but a caller may put it inside)
  case "$p" in "$aside"/*|"$aside") continue ;; esac
  # 1. a copy ALWAYS goes aside first, whatever happens next
  if [ -e "$p" ]; then
    mkdir -p "$aside/$(dirname "$p")" 2>/dev/null
    cp -a "$p" "$aside/$p" 2>/dev/null && kept=$((kept+1))
  fi
  # 2. then the tree is returned to HEAD, without `git checkout --`
  case "$st" in
    "??")
      mkdir -p "$aside/untracked/$(dirname "$p")" 2>/dev/null
      mv "$p" "$aside/untracked/$p" 2>/dev/null ;;
    "D "*)
      git show "HEAD:$p" > "$p" 2>/dev/null ;;
    *)
      [ -f "$p" ] && git show "HEAD:$p" > "$p" 2>/dev/null ;;
  esac
done < "$aside/status.txt"

left=$(git status --porcelain | wc -l)
printf '%s\n' "$aside" > "$aside/ASIDE.txt"
if [ "$left" -ne 0 ]; then
  echo "STILL-DIRTY $left path(s) remain; nothing was deleted, see $aside" >&2
  exit 3
fi
[ "$kept" -gt 0 ] && printf 'QUARANTINED %d\n%s\n' "$kept" "$aside" || printf 'QUARANTINED 0\n%s\n' "$aside"
