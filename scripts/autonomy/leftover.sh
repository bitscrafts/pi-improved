#!/usr/bin/env bash
# leftover.sh — the one reader for a dirty tree the loop finds at a cycle start (spec 311).
#   save <dir> <cid>  -> <cid>-staged.patch (git diff --cached), <cid>-tracked.patch (git diff),
#                        <cid>-untracked.tar (only with untracked paths),
#                        <cid>-paths.txt (git status -z: a rename lists BOTH paths). Creates files in <dir> only.
#   owner             -> OWNER <row-id> (exit 0) when exactly one open row with a session names a dirty path;
#                        FOREIGN <n> (exit 7) otherwise. n is the number of dirty paths (0 = clean).
#   clean <cid>       -> save first (PII_LEFTOVER_DIR, default _tmp/autonomy/leftovers), then
#                        git stash push --include-untracked (restore: git stash pop <ref>); NOT-CLEANED (exit 8)
#                        otherwise -- and NOT-CLEANED when the save failed, because a stash must never be the only copy.
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
  # The INDEX is content too, and `git diff` does not show it. Measured 2026-11-02: a person staged an edit, `save`
  # wrote nothing for it (not in `git diff`, and a staged file is not "others"), and `clean` then stashed it -- the
  # only copy was a stash commit that `git stash drop` removes in one word.
  if ! git diff --cached --quiet; then
    git diff --cached >"$dir/$cid-staged.patch" || return 1
    echo "SAVED $dir/$cid-staged.patch"
  fi
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
    # Opus review 2026-11-01: this used to count a row the moment ONE dirty path matched it. A row that names two
    # files of ten said OWNER, and clean() then stashed the other eight -- other people's work. A row owns this
    # tree only if EVERY dirty path is one it names; anything else is FOREIGN and must be left where it is.
    hit=1; IFS=';' read -ra cols <<<"$paths"
    for d in "${dirty[@]}"; do
      found=0
      for c in "${cols[@]}"; do [ "$c" = "$d" ] && { found=1; break; }; done
      [ "$found" = 1 ] || { hit=0; break; }
    done
    [ "$hit" = 1 ] && { hits=$((hits + 1)); who=$id; }
  done <<<"$ROWS"
  if [ "$n" -gt 0 ] && [ "$hits" -eq 1 ]; then echo "OWNER $who"; return 0; fi
  echo "FOREIGN $n"
  return 7
}

# foreign_paths <row-id>: the dirty paths that row does NOT name. Empty when the row owns the whole tree, which is
# the only case clean() may act on. This is what makes the refusal nameable instead of just "FOREIGN 10".
foreign_paths(){
  local PLAN=${PII_PLAN:-$ROOT/scripts/autonomy/plan.tsv} want c d found
  local -a cols dirty
  want=$(awk -F'\t' -v id="$1" '!/^#/ && $1 == id {print $6; exit}' "$PLAN" 2>/dev/null)
  IFS=';' read -ra cols <<<"$want"
  mapfile -t dirty < <(dirty_paths)
  for d in "${dirty[@]}"; do
    found=0
    for c in "${cols[@]}"; do [ "$c" = "$d" ] && { found=1; break; }; done
    [ "$found" = 1 ] || printf '%s\n' "$d"
  done
}

# still_dirty: the dirty paths left after an action, minus the leftovers dir this script writes into. Without
# this, clean() reports its own record as unfinished work (measured: `_tmp/autonomy/leftovers/` named as
# "1 path(s) still dirty" on a tree that was in fact clean).
still_dirty(){
  local left=${PII_LEFTOVER_DIR:-$LEFT_DEFAULT} rel p
  rel=${left#"$ROOT/"}
  # --untracked-files=all: git collapses a wholly-untracked directory to `dir/`, which hid the leftovers dir
  # behind its parent and made the parent look like unfinished work.
  git status --porcelain --untracked-files=all | while IFS= read -r rec; do
    p=${rec:3}
    case "$p" in "$rel"|"$rel"/|"$rel"/*) continue ;; esac
    printf '%s\n' "$p"
  done
}

clean(){
  local cid=$1 REF WHO FOREGN OWNED before after
  if [ -z "$(git status --porcelain)" ]; then echo "CLEAN nothing to stash"; return 0; fi
  # A stash is a tree-wide act. It may only happen when ONE row names every dirty path, and even then it carries
  # only that row's paths, so an unrelated edit can never disappear into it.
  WHO=$(owner) || true
  case "$WHO" in
    OWNER\ *) OWNED=${WHO#OWNER } ;;
    *) echo "REFUSED $WHO -- the tree holds work no single open row names; a tree-wide stash would swallow it"
       dirty_paths | sed 's/^/  /'
       # The VERDICT is its own line: a caller (and the test that guards this) greps one word, while the reason
       # above says why. Before this, a refusal printed only REFUSED and the documented NOT-CLEANED never appeared.
       echo "NOT-CLEANED"
       return 8 ;;
  esac
  if [ -n "$(foreign_paths "$OWNED")" ]; then
    echo "REFUSED OWNER $OWNED does not name every dirty path"
    foreign_paths "$OWNED" | sed 's/^/  /'
    echo "NOT-CLEANED"
    return 8
  fi
  IFS=$'\n' read -r -d '' -a PATHS < <(dirty_paths; printf '\0')
  before=$(dirty_paths | wc -l)
  # The stash must not be the ONLY copy: save the content where a `git stash drop` cannot reach it, and refuse to
  # stash when that save did not happen. A stash is a convenience; a file in the leftovers dir is the record.
  if ! save "${PII_LEFTOVER_DIR:-$LEFT_DEFAULT}" "$cid"; then
    echo "NOT-CLEANED the content could not be saved; refusing to put it only in a stash"
    dirty_paths | sed 's/^/  /'
    return 8
  fi
  if git stash push --include-untracked -m "pii-leftover-$cid" -- "${PATHS[@]}" >/dev/null 2>&1; then
    after=$(still_dirty | wc -l)
    REF=$(git stash list --format='%gd %gs' | grep -m1 -F "pii-leftover-$cid" | cut -d' ' -f1)
    echo "STASH $REF of $OWNED ($before path(s))  (restore: git stash pop $REF)"
    [ "$after" -eq 0 ] && return 0
    echo "INCOMPLETE: $after path(s) still dirty"; still_dirty | sed 's/^/  /'; return 8
  fi
  echo "NOT-CLEANED"
  dirty_paths | sed 's/^/  /'
  return 8
}

list(){
  local dir=${1:-$LEFT_DEFAULT} f cid bytes n seen=" " g
  [ -d "$dir" ] || return 0
  for f in $(ls -1tr "$dir"); do
    case "$f" in *-tracked.patch|*-staged.patch|*-untracked.tar|*-paths.txt) ;; *) continue ;; esac
    cid=${f%-tracked.patch}; cid=${cid%-staged.patch}; cid=${cid%-untracked.tar}; cid=${cid%-paths.txt}
    case "$seen" in *" $cid "*) continue ;; esac
    seen="$seen$cid "
    bytes=0; n=0
    for g in "$dir/$cid-tracked.patch" "$dir/$cid-staged.patch" "$dir/$cid-untracked.tar" "$dir/$cid-paths.txt"; do
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
