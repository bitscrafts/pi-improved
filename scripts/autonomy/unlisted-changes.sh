#!/usr/bin/env bash
# unlisted-changes.sh <paths> — prints each changed path the row's path list does not name (spec 268).
# <paths> is the row's own format: ';'-separated, a listed directory covers everything beneath it.
# Exit 1 when it printed any path, 0 when it printed none. Reads git status only: it never stages, commits or resets.
set -u
IFS=';' read -ra L <<<"${1:-}"
N=0
while IFS= read -r LINE; do
  ST=${LINE:0:2}; P=${LINE:3}
  # modified, deleted or untracked only (a staged-and-clean entry is the row's own work)
  case "$ST" in "??"|?[MD]) ;; *) continue ;; esac
  # the queue's own files: HANDOFF.md, the __pycache__ of the autonomy scripts, QUEUE-BRIEF*
  case "$P" in HANDOFF.md|scripts/autonomy/__pycache__/*) continue ;; esac
  case "$(basename "$P")" in QUEUE-BRIEF*) continue ;; esac
  COVERED=""
  for E in "${L[@]}"; do
    E=${E%/}
    case "$P" in "$E"|"$E"/*) COVERED=1; break ;; esac
  done
  [ -n "$COVERED" ] && continue
  echo "$P"; N=$((N+1))
done < <(git status --porcelain --untracked-files=all)
[ "$N" -eq 0 ]
