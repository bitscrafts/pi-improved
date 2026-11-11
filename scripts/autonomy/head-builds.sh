#!/usr/bin/env bash
# head-builds.sh — does the CONTENT of HEAD compile? (spec 245)
#   head-builds.sh              git archive HEAD into a throwaway export, cargo check --workspace there
#   head-builds.sh --worktree   the same after overlaying modified + untracked-not-ignored files (the pre-commit door)
# Prints "HEAD <sha> builds" (exit 0) or "HEAD <sha> does NOT build" + the first error lines (exit 1).
# No network, no claude, no gate. Removes only the mktemp -d directory it made in this run, after the verdict.
. "$(dirname "$(readlink -f "$0")")/env.sh" 2>/dev/null || true
set -uo pipefail
ROOT=${HEAD_BUILDS_REPO:-$PII_ROOT}
BUILD=$(PII_ROOT=$ROOT; . "$(dirname "$(readlink -f "$0")")/gate-env.sh"; gate_build_check) || { echo "head-builds: the .pii/gate of '$ROOT' is refused" >&2; exit 2; }
cd "$ROOT" || exit 2
mkdir -p "$ROOT/_tmp"
X=$(mktemp -d "$ROOT/_tmp/head-builds-XXXXXX") || exit 2
case "$X" in "$ROOT"/_tmp/head-builds-?*) ;; *) echo "head-builds: refusing odd scratch '$X'" >&2; exit 2 ;; esac
SHA=$(git rev-parse --short HEAD)
# -m: archived mtimes are the commit time, so two commits a second apart would share a cargo fingerprint
git archive HEAD | tar -x -m -C "$X" || { echo "head-builds: export failed" >&2; exit 2; }
if [ "${1:-}" = "--worktree" ]; then
  git ls-files -z -m -o --exclude-standard | while IFS= read -r -d '' f; do [ -e "$f" ] && printf '%s\0' "$f"; done \
    | tar --null -T - -c -f - | tar -x -m -C "$X"
fi
TARGET=${HEAD_BUILDS_TARGET:-$ROOT/_tmp/head-builds-target}
ERR="$X/.head-builds.err"
( cd "$X" && CARGO_TARGET_DIR="$TARGET" bash -c "$BUILD" ) >"$ERR" 2>&1
RC=$?
if [ "$RC" -eq 0 ]; then
  echo "HEAD $SHA builds"
else
  echo "HEAD $SHA does NOT build"
  grep -E -A3 '^error' "$ERR" | head -20
fi
rm -rf -- "$X"
[ "$RC" -eq 0 ]
