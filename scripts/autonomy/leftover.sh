#!/usr/bin/env bash
# leftover.sh — the one reader for a dirty tree the loop finds at a cycle start (spec 311).
#   save <dir> <cid>  -> <cid>-tracked.patch (git diff), <cid>-untracked.tar (only with untracked paths),
#                        <cid>-paths.txt (git status -z: a rename lists BOTH paths). Creates files in <dir> only.
#   owner             -> OWNER <row-id> (exit 0) when exactly one open row with a session names a dirty path;
#                        FOREIGN <n> (exit 7) otherwise. n is the number of dirty paths (0 = clean).
#   clean <cid>       -> git stash push --include-untracked (restore: git stash pop <ref>); NOT-CLEANED (exit 8) otherwise.
#   list [dir]        -> one line per saved leftover, newest last: <cid> <bytes> <paths-n>.
# Works on the git repository in the current directory. No checkout of paths, no hard reset, no forced clean:
# the content must stay recoverable, and a stash commit is.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
LEFT_DEFAULT=$ROOT/_tmp/autonomy/leftovers

usage(){ echo "usage: leftover.sh save <dir> <cid> | owner | clean <cid> | list [dir]" >&2; exit 2; }

# dirty_paths: every path git status reports, one per line; -z is NUL-split, and a rename or copy record is
# followed by its origin path, so both halves of a rename are listed
dirty_paths(){
  local rec
  git status --porcelain -z | while IFS= read -r -d '' rec; do
    printf '%s\n' "${rec:3}"
    case "${rec:0:2}" in *R*|*C*) IFS= read -r -d '' rec && printf '%s\n' "$rec" ;; esac
  done
}

save(){
  local dir=$1 cid=$2 P
  mkdir -p "$dir" || return 1
  if ! git diff --quiet; then
    git diff >"$dir/$cid-tracked.patch" || return 1
    echo "SAVED $dir/$cid-tracked.patch"
  fi
  if [ -n "$(git ls-files --others --exclude-standard)" ]; then
    git ls-files --others --exclude-standard -z | tar --null -cf "$dir/$cid-untracked.tar" -T - || return 1
    echo "SAVED $dir/$cid-untracked.tar"
  fi
  P=$(dirty_paths)
  if [ -n "$P" ]; then
    printf '%s\n' "$P" >"$dir/$cid-paths.txt"
    echo "SAVED $dir/$cid-paths.txt"
  fi
  return 0
}

# owner: bash only (no python here). The plan's column 6 and the state's status and session are read as state.py
# writes them: the state is indent=1 with sorted keys, so each row's status and session sit on their own lines.
owner(){
  local PLAN=${PII_PLAN:-$ROOT/scripts/autonomy/plan.tsv} STATE=${PII_STATE:-$ROOT/_tmp/autonomy/state.json}
  local -a dirty cols; local -A status session
  local ROWS STATE_ROWS sid sst sses id paths st ses c d hit hits=0 who="" n
  mapfile -t dirty < <(dirty_paths)
  n=${#dirty[@]}
  STATE_ROWS=$(awk -F'"' '/^ "/ && /\{$/ {id=$2} /^  "status"/ {s=$4} /^  "session"/ {e=$4}
    /^ \}/ {if (id != "") print id "\t" s "\t" e; id=""; s=""; e=""}' "$STATE" 2>/dev/null)
  while IFS=$'\t' read -r sid sst sses; do
    [ -n "$sid" ] && { status[$sid]=$sst; session[$sid]=$sses; }
  done <<<"$STATE_ROWS"
  ROWS=$(awk -F'\t' '!/^#/ && NF >= 7 {print $1 "\t" $6}' "$PLAN" 2>/dev/null)
  while IFS=$'\t' read -r id paths; do
    [ -n "$id" ] || continue
    st=${status[$id]:-pending}; ses=${session[$id]:-}
    case "$st" in done|skipped|revoked) continue ;; esac
    [ -n "$ses" ] || continue
    hit=0; IFS=';' read -ra cols <<<"$paths"
    for c in "${cols[@]}"; do
      for d in "${dirty[@]}"; do [ "$c" = "$d" ] && hit=1; done
    done
    [ "$hit" = 1 ] && { hits=$((hits + 1)); who=$id; }
  done <<<"$ROWS"
  if [ "$n" -gt 0 ] && [ "$hits" -eq 1 ]; then echo "OWNER $who"; return 0; fi
  echo "FOREIGN $n"
  return 7
}

clean(){
  local cid=$1 REF LEFT
  if [ -z "$(git status --porcelain)" ]; then echo "CLEAN nothing to stash"; return 0; fi
  if git stash push --include-untracked -m "pii-leftover-$cid" >/dev/null 2>&1 && [ -z "$(git status --porcelain)" ]; then
    REF=$(git stash list --format='%gd %gs' | grep -m1 -F "pii-leftover-$cid" | cut -d' ' -f1)
    echo "STASH $REF  (restore: git stash pop $REF)"
    return 0
  fi
  echo "NOT-CLEANED"
  dirty_paths | sed 's/^/  /'
  return 8
}

list(){
  local dir=${1:-$LEFT_DEFAULT} f cid bytes n seen=" " g
  [ -d "$dir" ] || return 0
  for f in $(ls -1tr "$dir"); do
    case "$f" in *-tracked.patch|*-untracked.tar|*-paths.txt) ;; *) continue ;; esac
    cid=${f%-tracked.patch}; cid=${cid%-untracked.tar}; cid=${cid%-paths.txt}
    case "$seen" in *" $cid "*) continue ;; esac
    seen="$seen$cid "
    bytes=0; n=0
    for g in "$dir/$cid-tracked.patch" "$dir/$cid-untracked.tar" "$dir/$cid-paths.txt"; do
      [ -f "$g" ] && bytes=$((bytes + $(wc -c <"$g")))
    done
    [ -f "$dir/$cid-paths.txt" ] && n=$(wc -l <"$dir/$cid-paths.txt")
    echo "$cid $bytes $n"
  done
}

case "${1:-}" in
  save)  [ $# -eq 3 ] || usage; save "$2" "$3" ;;
  owner) owner ;;
  clean) [ $# -eq 2 ] || usage; clean "$2" ;;
  list)  list "${2:-}" ;;
  *)     usage ;;
esac
