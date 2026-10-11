#!/usr/bin/env bash
# Control for the "no test opens a store at a path it does not own" class (spec 134).
# Usage: concurrent-store.sh <package> <test-binary> [rounds]
# Runs the same test binary twice at the same moment, <rounds> times, and prints one line
# `ROUNDS <n> COLLISIONS <k>` (k = rounds where any process reported a lock/open failure).
# Exit 0 only when k = 0. Scratch (logs) lives in its own mktemp dir under /projects/_tmp.
set -u
pkg="${1:?usage: concurrent-store.sh <package> <test-binary> [rounds]}"
bin_name="${2:?usage: concurrent-store.sh <package> <test-binary> [rounds]}"
rounds="${3:-10}"
root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root" || exit 2

# Build once and resolve the test executable (never `cargo test` twice: the build lock would serialise it).
exe="$(cargo test -p "$pkg" --test "$bin_name" --no-run --message-format=json 2>/dev/null |
  grep -o '"executable":"[^"]*"' | tail -1 | cut -d'"' -f4)"
if [ -z "$exe" ] || [ ! -x "$exe" ]; then
  echo "ROUNDS 0 COLLISIONS 0 (could not build $pkg/$bin_name)" >&2
  exit 2
fi
# cargo runs a crate's tests with cwd = the crate dir; do the same.
crate_dir="$(dirname "$(cargo metadata --no-deps --format-version 1 2>/dev/null |
  grep -o "\"name\":\"$pkg\",[^}]*\"manifest_path\":\"[^\"]*\"" | grep -o '"manifest_path":"[^"]*"' | cut -d'"' -f4)")"
[ -d "$crate_dir" ] || { echo "ROUNDS 0 COLLISIONS 0 (no crate dir for $pkg)" >&2; exit 2; }

scratch="$(mktemp -d /projects/_tmp/concurrent-store.XXXXXX)" || exit 2
collisions=0
n=1
while [ "$n" -le "$rounds" ]; do
  (cd "$crate_dir" && "$exe" >"$scratch/a.$n" 2>&1) &
  pa=$!
  (cd "$crate_dir" && "$exe" >"$scratch/b.$n" 2>&1) &
  pb=$!
  wait "$pa"; ra=$?
  wait "$pb"; rb=$?
  if grep -qiE 'Database already open|Cannot acquire lock|DatabaseAlreadyOpen' "$scratch/a.$n" "$scratch/b.$n" ||
    [ "$ra" -ne 0 ] || [ "$rb" -ne 0 ]; then
    collisions=$((collisions + 1))
  fi
  n=$((n + 1))
done
echo "logs: $scratch" >&2
echo "ROUNDS $rounds COLLISIONS $collisions"
[ "$collisions" -eq 0 ]
