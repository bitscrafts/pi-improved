#!/usr/bin/env bash
# tidy-tmp.sh — bound /projects/_tmp, the WORKSPACE scratch area.
#
# MEASURED 2026-11-02: the workspace `_tmp` held 11 GB, 3268 files, 713 directories and **1598 top-level `*.sh`**
# — one script per step of every past session, almost none reusable. The user's words: *"the _tmp folder must be
# deeply polluted! everytime you do anything you create a script! withut any resusage!"*
#
# WHAT IT DOES (and deliberately does not):
#   1. moves top-level `*.sh` into `<trash>/_tmp-scripts-<stamp>/` with a manifest — reversible, never deleted;
#   2. deletes `target/` inside `_tmp/*/` — BUILD OUTPUT of scratch copies; the trees themselves stay;
#   3. never touches live state (`autonomy`, `claude-batchC`, `digest-proof`, `deploy`) or the repo at all.
#
# THE BUG THIS SCRIPT WAS BORN WITH, kept as a warning: its first version resolved `../..` to the REPO and
# proposed deleting 174 `target/` dirs INSIDE /projects/pi-improved/_tmp — including the 75 GB warm gate cache and
# the loop's live state. A dry run caught it. The guards below make that impossible rather than unlikely.
#
# Usage: tidy-tmp.sh [--dry-run|--apply]   (default --dry-run)
set -uo pipefail

APPLY=0
case "${1:-}" in
  --apply) APPLY=1 ;;
  --dry-run|"") APPLY=0 ;;
  *) echo "usage: $0 [--dry-run|--apply]" >&2; exit 2 ;;
esac

REPO=$(cd "$(dirname "$0")/../.." && pwd)
WORKSPACE=$(cd "$REPO/.." && pwd)
TMPD="$WORKSPACE/_tmp"          # /projects/_tmp — the SCRATCH area, NOT the repo's own _tmp
TRASH="$WORKSPACE/_trash"
[ -d "$TMPD" ] || { echo "tidy-tmp: no such directory: $TMPD" >&2; exit 3; }

case "$TMPD" in
  "$REPO"/*) echo "tidy-tmp: REFUSED — $TMPD is inside the repo" >&2; exit 4 ;;
esac
if [ "$APPLY" -eq 1 ]; then
  case "$TMPD" in
    */_tmp) : ;;
    *) echo "tidy-tmp: REFUSED — $TMPD is not a workspace _tmp" >&2; exit 4 ;;
  esac
fi

STAMP=$(date -u +%Y%m%dT%H%M%SZ)
DEST="$TRASH/_tmp-scripts-$STAMP"

echo "tidy-tmp: $TMPD"
echo "  before: $(du -sh "$TMPD" 2>/dev/null | cut -f1), $(find "$TMPD" -maxdepth 1 -type f | wc -l) files, $(find "$TMPD" -maxdepth 1 -mindepth 1 -type d | wc -l) dirs"

scripts=$(find "$TMPD" -maxdepth 1 -type f -name '*.sh' | wc -l)
echo "  scripts to move: $scripts -> $DEST"
if [ "$APPLY" -eq 1 ] && [ "$scripts" -gt 0 ]; then
  mkdir -p "$DEST"
  find "$TMPD" -maxdepth 1 -type f -name '*.sh' -printf '%f\n' | sort > "$DEST/MANIFEST.txt"
  find "$TMPD" -maxdepth 1 -type f -name '*.sh' -exec mv -t "$DEST" {} + 2>/dev/null
  echo "  moved: $(find "$DEST" -maxdepth 1 -name '*.sh' | wc -l) script(s), manifest at $DEST/MANIFEST.txt"
fi

# Build output only: `target/` inside scratch copies. Live state and the gate cache are never candidates.
# Build output, under every name this workspace has given it: `target` (cargo), `tgt` (a custom target dir), and a
# nested `_tmp` inside a scratch COPY (cast-review keeps its build there). The rule is a BASENAME test plus an
# exclusion list — the first version used path patterns and `-prune` without `-print`, so it silently found nothing
# while 85 directories qualified. The trees themselves are never touched.
maps=()
while IFS= read -r t; do
  [ "$t" = "$TMPD" ] && continue
  case "$t" in
    */autonomy/*|*/claude-batchC/*|*/gate-target*|*/digest-proof/*|*/deploy/*) continue ;;
  esac
  case "$(basename "$t")" in
    target|tgt|_tmp) maps+=("$t") ;;
  esac
done < <(find "$TMPD" -maxdepth 3 -type d \( -name target -o -name tgt -o -name _tmp \) -print 2>/dev/null)

total=0
for t in ${maps[@]+"${maps[@]}"}; do total=$(( total + $(du -sk "$t" 2>/dev/null | cut -f1) )); done
echo "  build output to drop: ${#maps[@]} target/ dir(s), $(( total / 1024 )) MB"
if [ "$APPLY" -eq 1 ] && [ "${#maps[@]}" -gt 0 ]; then
  : > "$TRASH/tidy-tmp-$STAMP.log"
  for t in "${maps[@]}"; do
    printf '%s removed %s (%s KB)\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$t" "$(du -sk "$t" 2>/dev/null | cut -f1)" >> "$TRASH/tidy-tmp-$STAMP.log"
    rm -rf -- "$t"
  done
  echo "  freed: $(( total / 1024 )) MB (record: $TRASH/tidy-tmp-$STAMP.log)"
fi

echo "  after : $(du -sh "$TMPD" 2>/dev/null | cut -f1), $(find "$TMPD" -maxdepth 1 -type f | wc -l) files, $(find "$TMPD" -maxdepth 1 -mindepth 1 -type d | wc -l) dirs"
