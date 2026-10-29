#!/usr/bin/env bash
# gate-touched-crates.sh — the tests of the workspace packages a change touches (spec 314).
# --print: prints the one cargo command it would run; --run: runs it and exits with its status.
# Paths after the flag are the change; with none, the change is what `git status` reports in the repo.
# GATE_REPO overrides the repo (the tests point it at a scratch repo). Writes nothing, deletes nothing.
# Every "nothing to do" is a printed line on stderr, so a silent green is impossible.
set -u
ROOT="${GATE_REPO:-$(cd "$(dirname "$0")/.." && pwd)}"
case "${1:-}" in --print | --run) MODE=$1; shift ;; *) echo "usage: gate-touched-crates.sh --print|--run [path...]" >&2; exit 2 ;; esac
PKGS=""
BAD=0

# pkg_name <Cargo.toml>: the package name, the first `name =` inside [package]; empty if none
pkg_name(){ [ -f "$1" ] && awk '/^\[/{p = ($0 == "[package]")} p && /^name *=/{gsub(/.*= *"|".*/, ""); print; exit}' "$1"; }
# all_members: every package in the workspace — each crates/<dir> plus the root xtask
all_members(){
  local d n
  for d in "$ROOT"/crates/*/; do n=$(pkg_name "$d/Cargo.toml"); [ -n "$n" ] && echo "$n"; done
  echo xtask
}
# add_path <repo path>: the packages that own it, appended to PKGS; an unresolvable crates/ path sets BAD
add_path(){
  local d n
  case "$1" in
    crates/*/*)
      d=${1#crates/}; d=${d%%/*}
      n=$(pkg_name "$ROOT/crates/$d/Cargo.toml")
      if [ -z "$n" ]; then echo "gate-touched-crates: $1 names no workspace package" >&2; BAD=1; return; fi
      PKGS="$PKGS $n" ;;
    xtask/*) PKGS="$PKGS xtask" ;;
    Cargo.toml | Cargo.lock) PKGS="$PKGS $(all_members | tr '\n' ' ')" ;;
  esac
}

if [ $# -gt 0 ]; then
  for p in "$@"; do add_path "$p"; done
else
  if [ ! -e "$ROOT/.git" ]; then echo "gate-touched-crates: no git repository — nothing to test here" >&2; exit 0; fi
  git -C "$ROOT" status --porcelain=v1 -z >/dev/null || { echo "gate-touched-crates: git status failed" >&2; exit 2; }
  # -z entries are "XY path" NUL-terminated; a rename or copy is followed by its origin path, which is touched too
  orig=0
  while IFS= read -r -d '' entry; do
    if [ "$orig" = 1 ]; then add_path "$entry"; orig=0; continue; fi
    add_path "${entry:3}"
    case "${entry:0:2}" in *R* | *C*) orig=1 ;; esac
  done < <(git -C "$ROOT" status --porcelain=v1 -z)
fi
[ "$BAD" = 0 ] || exit 2

LIST=$(printf '%s\n' $PKGS | sed '/^$/d' | sort -u)
if [ -z "$LIST" ]; then echo "gate-touched-crates: no crate touched" >&2; exit 0; fi
CMD=(cargo test -j 2)
for p in $LIST; do CMD+=(-p "$p"); done
CMD+=(--lib --bins --tests)

if [ "$MODE" = --print ]; then echo "${CMD[*]}"; exit 0; fi
echo "gate-touched-crates: ${CMD[*]}"
start=$(date +%s)
(cd "$ROOT" && "${CMD[@]}")
rc=$?
echo "gate-touched-crates: took $(( $(date +%s) - start ))s (exit $rc)"
exit "$rc"
