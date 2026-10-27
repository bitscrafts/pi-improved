#!/usr/bin/env bash
# Spec 292 R2: the seam between the live writer and the reaper is covered by a test that reads a spawn a real run
# journalled. The control removes the pid from `journal_spawned`; the new test must go red, and the twelve seeded tests
# of `orphans_are_reaped.rs` must stay green (if one turns red, the seam was already covered: SEAM-ALREADY-COVERED).
# Usage: bash scripts/checks/live-spawn-reap.sh
# Prints one verdict line, `LIVE-SPAWN-REAP: OK` (exit 0) or `LIVE-SPAWN-REAP: FAIL <why>` (exit 1, or 3 for NO-APPLY).
# The tree is left byte-identical: the saved bytes are written back by a trap on every exit path.
set -u
root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root" || exit 1

src="crates/pii-server/src/node_kinds/nodes/coder.rs"
anchor='.field("pid", i64::from(s.pid))'
scratch="$(mktemp -d "${LIVE_SPAWN_SCRATCH:-/projects/_tmp}/live-spawn-reap.XXXXXX")" || {
  echo "LIVE-SPAWN-REAP: FAIL no scratch directory"
  exit 1
}
mutated=0
restore() {
  if [ "$mutated" -eq 1 ]; then
    cp "$scratch/coder.rs.orig" "$src" && touch "$src"
    mutated=0
  fi
}
trap restore EXIT

# 1. The digest before anything is planted.
before="$(sha256sum "$src" | cut -d' ' -f1)"
echo "BEFORE $before"

# 2. The saved bytes: a copy in scratch, never a hardlink or a move.
cp "$src" "$scratch/coder.rs.orig" || { echo "LIVE-SPAWN-REAP: FAIL could not save $src"; exit 1; }
cmp -s "$src" "$scratch/coder.rs.orig" || { echo "LIVE-SPAWN-REAP: FAIL saved copy differs"; exit 1; }

# 3. Plant the mutation: remove the single pid line. Refuse unless the anchor occurs exactly once.
hits="$(grep -cF -- "$anchor" "$src")"
if [ "$hits" -ne 1 ]; then
  echo "LIVE-SPAWN-REAP: FAIL NO-APPLY anchor occurs $hits times"
  exit 3
fi
ANCHOR="$anchor" perl -ne 'print unless index($_, $ENV{ANCHOR}) >= 0' "$src" > "$scratch/mutated.rs" || {
  echo "LIVE-SPAWN-REAP: FAIL could not write the mutation"
  exit 1
}
cp "$scratch/mutated.rs" "$src" && touch "$src"
mutated=1
echo "MUTATED removed: $anchor"

# 4. The new test must be red, on one of its own assertions.
cargo test -q -p pii-server --test live_spawn_is_reaped >"$scratch/red.log" 2>&1
red_rc=$?
if [ "$red_rc" -eq 0 ]; then
  echo "LIVE-SPAWN-REAP: FAIL the new test stayed green under the mutation"
  exit 1
fi
if ! grep -q 'panicked at' "$scratch/red.log"; then
  echo "LIVE-SPAWN-REAP: FAIL does not compile"
  exit 1
fi
echo "RED $(grep -m1 'panicked at .*live_spawn_is_reaped.rs' "$scratch/red.log")"

# 5. The seeded suite must be green on all twelve.
cargo test -q -p pii-server --test orphans_are_reaped >"$scratch/seeded.log" 2>&1
if ! grep -qE '^test result: ok\. 12 passed' "$scratch/seeded.log"; then
  echo "LIVE-SPAWN-REAP: SEAM-ALREADY-COVERED the seeded suite turned red under the mutation"
  exit 1
fi
echo "SEEDED $(grep -E '^test result' "$scratch/seeded.log" | head -1)"

# 6. Restore: the saved bytes back, then touch. Never `cp -p`: it keeps the old mtime and cargo reuses the mutant.
restore
echo "RESTORED"

# 7. The digest must match, and the new test must pass again.
after="$(sha256sum "$src" | cut -d' ' -f1)"
echo "AFTER $after"
if [ "$after" != "$before" ]; then
  echo "LIVE-SPAWN-REAP: FAIL restored digest differs"
  exit 1
fi
cargo test -q -p pii-server --test live_spawn_is_reaped >"$scratch/green.log" 2>&1 || {
  echo "LIVE-SPAWN-REAP: FAIL the new test is red after restore"
  exit 1
}
echo "GREEN $(grep -E '^test result' "$scratch/green.log" | head -1)"

# 8. The tree is byte-identical under crates/pii-server/src.
if [ -n "$(git status --porcelain crates/pii-server/src)" ]; then
  echo "LIVE-SPAWN-REAP: FAIL crates/pii-server/src is not clean"
  exit 1
fi
echo "LIVE-SPAWN-REAP: OK"
exit 0
