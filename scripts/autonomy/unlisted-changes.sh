#!/usr/bin/env bash
# unlisted-changes.sh <paths> — prints each changed path the row's path list does not name (spec 268).
# <paths> is the row's own format: ';'-separated, a listed directory covers everything beneath it.
# Exit 1 when it printed any path, 0 when it printed none. Reads git status only: it never stages, commits or resets.
set -u
IFS=';' read -ra L <<<"${1:-}"
N=0
# -z: NUL-separated and unquoted (spec 277). A rename or copy is "R  new" then "old" as its own field: the new path is the change
while IFS= read -r -d '' LINE; do
  ST=${LINE:0:2}; P=${LINE:3}
  case "$ST" in *[RC]*) IFS= read -r -d '' _OLD ;; esac
  # the queue's own files: HANDOFF.md, the __pycache__ of the autonomy scripts, QUEUE-BRIEF*
  case "$P" in HANDOFF.md|scripts/autonomy/__pycache__/*) continue ;; esac
  case "${P##*/}" in QUEUE-BRIEF*) continue ;; esac
  COVERED=""
  for E in "${L[@]}"; do
    E=${E%/}
    case "$P" in "$E"|"$E"/*) COVERED=1; break ;; esac
  done
  [ -n "$COVERED" ] && continue
  printf '%s\n' "$P"; N=$((N+1))
done < <(git status --porcelain=v1 -z --untracked-files=all)
[ "$N" -eq 0 ]
