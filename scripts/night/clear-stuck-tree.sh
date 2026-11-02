#!/usr/bin/env bash
# clear-stuck-tree.sh <destination> [--apply] — move a tree nobody's row owns into a dated directory under _trash.
#
# WHY. A worker that fails leaves its partial work in the tree. Every row behind it is then refused with
# `dirty-tree`, and the night stops advancing even though the queue is full. The fix must not be a delete and must
# not be a silent `git checkout --`: the worker's work is evidence, and a person may want it.
#
# THE RULES THIS OBEYS:
#   · nothing is deleted: every path is `mv`-d, and a MANIFEST.txt records what moved and from where;
#   · `git` is never asked to discard anything (no checkout, no reset, no clean);
#   · it refuses when the tree holds a path a *live* row names (that is that row's work, not garbage);
#   · it refuses without --apply, and prints what it would move.
#
# Usage: clear-stuck-tree.sh _trash/night-stuck-<stamp> [--apply]
set -uo pipefail
cd /projects/pi-improved || exit 1
DEST=${1:?usage: clear-stuck-tree.sh <destination-dir> [--apply]}
APPLY=0; [ "${2:-}" = "--apply" ] && APPLY=1

# The destination must be absolute and outside the repository. A relative `_trash/…` creates an untracked directory
# INSIDE the tree — the very dirt being cleared — which is how this bit us at 07:25 on 2026-11-02.
case "$DEST" in
  /*) ;;
  *) echo "REFUSED: the destination must be an ABSOLUTE path under /projects/_trash (got $DEST)"; exit 2 ;;
esac
case "$DEST" in
  /projects/_trash/*) ;;
  *) echo "REFUSED: the destination must be under /projects/_trash (got $DEST)"; exit 2 ;;
esac

mapfile -t PATHS < <(git status --porcelain -z | while IFS= read -r -d '' rec; do printf '%s\n' "${rec:3}"; done)
[ "${#PATHS[@]}" -gt 0 ] || { echo "CLEAN nothing to move"; exit 0; }

# A dirty path that a dispatchable row DECLARES is that row's work: refuse rather than move it. Being a valid row
# is not enough — every row in the queue is valid, and refusing on that basis would refuse always (measured
# 2026-11-02: the first version did exactly that, because the head row was a different row's work entirely).
owner=$(python3 scripts/autonomy/state.py pick 2>/dev/null | head -1)
if [ -n "$owner" ] && [ "$owner" != "NONE" ]; then
  rid=${owner%%$'\t'*}
  want=$(awk -F'\t' -v id="$rid" '$1==id {print $6; exit}' scripts/autonomy/plan.tsv)
  hit=""
  IFS=';' read -ra declared <<<"$want"
  for p in "${PATHS[@]}"; do
    for d in "${declared[@]}"; do
      [ -n "$d" ] && [ "$p" = "$d" ] && hit=$p
    done
  done
  if [ -n "$hit" ]; then
    echo "REFUSED: row $rid declares $hit, which is dirty — that is its work, not garbage"
    exit 3
  fi
fi

echo "would move ${#PATHS[@]} path(s) to $DEST:"
printf '  %s\n' "${PATHS[@]}"
[ "$APPLY" = 1 ] || { echo "(dry run: pass --apply to move them)"; exit 0; }

mkdir -p "$DEST" || { echo "REFUSED: cannot create $DEST"; exit 2; }
{
  echo "# moved by clear-stuck-tree.sh at $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "# HEAD $(git log --format='%h %s' -1)"
  echo "# row in front: ${owner:-none}"
  echo "# restore: for each pair, move <dest>/<path> back to <repo>/<path>"
} >"$DEST/MANIFEST.txt"
moved=0
for p in "${PATHS[@]}"; do
  [ -e "$p" ] || continue
  mkdir -p "$DEST/$(dirname "$p")"
  mv -- "$p" "$DEST/$p" 2>/dev/null && { echo "$p" >>"$DEST/MANIFEST.txt"; moved=$((moved+1)); }
done
echo "moved $moved path(s) to $DEST (manifest: $DEST/MANIFEST.txt)"
git status --porcelain | wc -l | xargs -I{} echo "tree now: {} changed path(s)"
