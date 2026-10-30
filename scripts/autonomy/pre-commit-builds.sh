#!/usr/bin/env bash
# pre-commit-builds.sh — does the PENDING COMMIT compile? (spec 315)
#   pre-commit-builds.sh          export HEAD, apply the INDEX onto it, cargo metadata --locked, then cargo check --workspace
#   pre-commit-builds.sh --print  the tree it would judge, the commands, and the target dir; runs nothing
# The object under test is the index, never the worktree: a modification present only in the worktree is not gated.
# Exit 0 green (or nothing staged), 1 the pending tree does not build, 2 the check could not run. No network, no git
# write, no gate. Removes only the mktemp -d directory it made in this run.
. "$(dirname "$(readlink -f "$0")")/env.sh" 2>/dev/null || true
set -uo pipefail
ROOT=${PRE_COMMIT_REPO:-/projects/pi-improved}
cd "$ROOT" 2>/dev/null || { echo "pre-commit: repository '$ROOT' is not readable"; exit 2; }
PRINT=0
case "${1:-}" in "") ;; --print) PRINT=1 ;; *) echo "pre-commit: unknown argument '$1'"; exit 2 ;; esac
TARGET=${HEAD_BUILDS_TARGET:-$ROOT/_tmp/head-builds-target}
SHA=$(git rev-parse --short HEAD 2>/dev/null) || { echo "pre-commit: no HEAD commit in $ROOT"; exit 2; }
# NUL-separated index paths, counted by their terminators; pipefail makes a failed git read a failed count
count(){ tr -cd '\0' | wc -c | tr -d ' '; }
ADDN=$(git diff --cached --no-renames --diff-filter=ACMR --name-only -z | count) || { echo "pre-commit: cannot read the index"; exit 2; }
DELN=$(git diff --cached --no-renames --diff-filter=D --name-only -z | count) || { echo "pre-commit: cannot read the index"; exit 2; }
if [ "$ADDN" -eq 0 ] && [ "$DELN" -eq 0 ]; then
  echo "pre-commit: nothing staged — nothing to gate"; exit 0
fi
if [ "$PRINT" -eq 1 ]; then
  echo "pre-commit: would judge HEAD $SHA + $ADDN staged path(s), $DELN deleted"
  echo "pre-commit: the index is applied onto the export; the worktree is never read"
  echo "pre-commit: cargo metadata --locked --format-version 1"
  echo "pre-commit: cargo check -q --workspace"
  echo "pre-commit: CARGO_TARGET_DIR=$TARGET (warm: reused across items, ~8s; a cold dir is ~38s)"
  exit 0
fi
command -v cargo >/dev/null 2>&1 || { echo "pre-commit: cargo is not on PATH"; exit 2; }
mkdir -p "$ROOT/_tmp"
X=$(mktemp -d "$ROOT/_tmp/pre-commit-XXXXXX") || { echo "pre-commit: cannot make a scratch directory"; exit 2; }
case "$X" in "$ROOT"/_tmp/pre-commit-?*) ;; *) echo "pre-commit: refusing odd scratch '$X'"; exit 2 ;; esac
trap 'case "$X" in "$ROOT"/_tmp/pre-commit-?*) rm -r -- "$X" ;; esac' EXIT
T0=$(date +%s)
mkdir -p "$X/tree"
# -m: archived mtimes are the commit time, so two commits a second apart would share a cargo fingerprint
git archive HEAD | tar -x -m -C "$X/tree" || { echo "pre-commit: export of HEAD failed"; exit 2; }
git diff --cached --no-renames --diff-filter=ACMR --name-only -z >"$X/added" || { echo "pre-commit: cannot read the index"; exit 2; }
git diff --cached --no-renames --diff-filter=D --name-only -z >"$X/deleted" || { echo "pre-commit: cannot read the index"; exit 2; }
# the index's content, not the worktree's: checkout-index writes only under --prefix and never touches the index
git checkout-index -f -z --stdin --prefix="$X/tree/" <"$X/added" || { echo "pre-commit: cannot read the index content"; exit 2; }
while IFS= read -r -d '' f; do rm -f -- "$X/tree/$f"; done <"$X/deleted"
ERR=$X/cargo.log
( cd "$X/tree" && cargo metadata --locked --format-version 1 ) >/dev/null 2>"$ERR"; RC=$?
if [ "$RC" -eq 0 ]; then
  ( cd "$X/tree" && CARGO_TARGET_DIR="$TARGET" cargo check -q --workspace ) >"$ERR" 2>&1; RC=$?
fi
if [ "$RC" -eq 0 ]; then
  echo "pre-commit: the pending tree builds ($(( $(date +%s) - T0 ))s)"; exit 0
fi
echo "pre-commit: the pending tree does NOT build"
grep -E '^error' "$ERR" | head -10
exit 1
