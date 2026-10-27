#!/usr/bin/env bash
# Spec 297 R2: the orphan count is read from a run a daemon wrote. The control shows the seam was uncovered by the seeded
# test: m1 removes the pid from `journal_spawned` (the write side), and the new test must go red while the seeded
# `orphan_is_countable` stays green; m2 zeroes the orphan increment in `store_totals` (the read side), and the new test must
# go red again. If the seeded test turns red under m1, the seam was already covered: SEAM-ALREADY-COVERED.
# Usage: bash scripts/checks/live-orphan-count.sh
# Prints one verdict line, `LIVE-ORPHAN-COUNT: OK` (exit 0) or `LIVE-ORPHAN-COUNT: FAIL <why>` (exit 1, or 3 for NO-APPLY).
# The tree is left byte-identical: the saved bytes are written back by a trap on every exit path.
set -u
root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root" || exit 1

writer="crates/pii-server/src/node_kinds/nodes/coder.rs"
reader="crates/pii-graph/src/agent_calls.rs"
writer_anchor='.field("pid", i64::from(s.pid))'
reader_anchor='totals.orphans_total += open;'
reader_plant='totals.orphans_total += 0;'

# The scratch lives inside the repository: the judge's sandbox refuses writes outside its own `_tmp`.
scratch_base="${LIVE_ORPHAN_SCRATCH:-$root/_tmp}"
mkdir -p "$scratch_base" || { echo "LIVE-ORPHAN-COUNT: FAIL no scratch base"; exit 1; }
scratch="$(mktemp -d "$scratch_base/live-orphan-count.XXXXXX")" || {
  echo "LIVE-ORPHAN-COUNT: FAIL no scratch directory"
  exit 1
}
mutated_w=0
mutated_r=0
restore() {
  if [ "$mutated_w" -eq 1 ]; then
    cp "$scratch/coder.rs.orig" "$writer" && touch "$writer"
    mutated_w=0
  fi
  if [ "$mutated_r" -eq 1 ]; then
    cp "$scratch/agent_calls.rs.orig" "$reader" && touch "$reader"
    mutated_r=0
  fi
}
trap restore EXIT
trap 'restore; exit 1' INT TERM

# Exits 1 unless the test log `$1` shows a panic in the file `$2`. A compile failure is named as such and is not a red.
require_red() {
  if grep -q 'could not compile' "$1"; then
    echo "LIVE-ORPHAN-COUNT: FAIL does not compile"
    exit 1
  fi
  if ! grep -q "panicked at .*$2" "$1"; then
    echo "LIVE-ORPHAN-COUNT: FAIL the new test failed without a panic in $2"
    exit 1
  fi
  echo "RED $(grep -m1 "panicked at .*$2" "$1")"
}

# 1. The digest of the writer before anything is planted, and its bytes saved into scratch (a copy, never a link or a move).
before_w="$(sha256sum "$writer" | cut -d' ' -f1)"
echo "BEFORE_W $before_w"
cp "$writer" "$scratch/coder.rs.orig" || { echo "LIVE-ORPHAN-COUNT: FAIL could not save $writer"; exit 1; }
cmp -s "$writer" "$scratch/coder.rs.orig" || { echo "LIVE-ORPHAN-COUNT: FAIL saved copy differs"; exit 1; }

# 2. m1, the write side: remove the single pid line. Refuse unless the anchor occurs exactly once, writing nothing.
hits="$(grep -cF -- "$writer_anchor" "$writer")"
if [ "$hits" -ne 1 ]; then
  echo "LIVE-ORPHAN-COUNT: FAIL NO-APPLY anchor occurs $hits times in $writer"
  exit 3
fi
ANCHOR="$writer_anchor" perl -ne 'print unless index($_, $ENV{ANCHOR}) >= 0' "$writer" > "$scratch/mutated.rs" || {
  echo "LIVE-ORPHAN-COUNT: FAIL could not write the m1 mutation"
  exit 1
}
cp "$scratch/mutated.rs" "$writer" && touch "$writer"
mutated_w=1
echo "MUTATED m1 removed: $writer_anchor"

# 3. The new test must be red on one of its own assertions, or on the panic that names the missing spawn.
cargo test -q -p pii-server --test live_orphan_is_countable >"$scratch/m1-new.log" 2>&1
if [ "$?" -eq 0 ]; then
  echo "LIVE-ORPHAN-COUNT: FAIL the new test stayed green under m1"
  exit 1
fi
require_red "$scratch/m1-new.log" "live_orphan_is_countable.rs"

# 4. The seeded test must stay green under m1: that green line is the measurement.
cargo test -q -p pii-server --test orphan_is_countable >"$scratch/m1-seeded.log" 2>&1
if ! grep -qE '^test result: ok\. 1 passed' "$scratch/m1-seeded.log"; then
  echo "LIVE-ORPHAN-COUNT: SEAM-ALREADY-COVERED the seeded test turned red under m1"
  exit 1
fi
echo "SEEDED $(grep -E '^test result' "$scratch/m1-seeded.log" | head -1)"

# 5. The spec 292 reaper test is expected red under m1 (the same pid line); it is printed so no reader mistakes it.
cargo test -q -p pii-server --test live_spawn_is_reaped >"$scratch/m1-reaped.log" 2>&1
if [ "$?" -eq 0 ]; then
  echo "LIVE-SPAWN-REAPED-GREEN under m1 (not expected)"
else
  echo "EXPECTED-RED live_spawn_is_reaped $(grep -m1 'panicked at' "$scratch/m1-reaped.log")"
fi

# 6. Restore m1: the saved bytes back, then touch. Never `cp -p`: it keeps the old mtime and cargo reuses the mutant.
restore
after_w="$(sha256sum "$writer" | cut -d' ' -f1)"
echo "AFTER_W $after_w"
if [ "$after_w" != "$before_w" ]; then
  echo "LIVE-ORPHAN-COUNT: FAIL restored digest of $writer differs"
  exit 1
fi

# 7. m2, the read side: zero the orphan increment. Refuse unless the anchor occurs exactly once.
before_r="$(sha256sum "$reader" | cut -d' ' -f1)"
echo "BEFORE_R $before_r"
cp "$reader" "$scratch/agent_calls.rs.orig" || { echo "LIVE-ORPHAN-COUNT: FAIL could not save $reader"; exit 1; }
cmp -s "$reader" "$scratch/agent_calls.rs.orig" || { echo "LIVE-ORPHAN-COUNT: FAIL saved copy differs"; exit 1; }
hits="$(grep -cF -- "$reader_anchor" "$reader")"
if [ "$hits" -ne 1 ]; then
  echo "LIVE-ORPHAN-COUNT: FAIL NO-APPLY anchor occurs $hits times in $reader"
  exit 3
fi
ANCHOR="$reader_anchor" PLANT="$reader_plant" perl -pe 's/\Q$ENV{ANCHOR}\E/$ENV{PLANT}/' "$reader" > "$scratch/agent_calls.mut.rs" || {
  echo "LIVE-ORPHAN-COUNT: FAIL could not write the m2 mutation"
  exit 1
}
cp "$scratch/agent_calls.mut.rs" "$reader" && touch "$reader"
mutated_r=1
echo "MUTATED m2 $reader_anchor -> $reader_plant"

# 8. The new test must be red under m2: it is not vacuous.
cargo test -q -p pii-server --test live_orphan_is_countable >"$scratch/m2-new.log" 2>&1
if [ "$?" -eq 0 ]; then
  echo "LIVE-ORPHAN-COUNT: FAIL the new test stayed green under m2"
  exit 1
fi
require_red "$scratch/m2-new.log" "live_orphan_is_countable.rs"

# 9. Restore m2, then the digest must match and the same target must pass again.
restore
after_r="$(sha256sum "$reader" | cut -d' ' -f1)"
echo "AFTER_R $after_r"
if [ "$after_r" != "$before_r" ]; then
  echo "LIVE-ORPHAN-COUNT: FAIL restored digest of $reader differs"
  exit 1
fi
cargo test -q -p pii-server --test live_orphan_is_countable >"$scratch/green.log" 2>&1 || {
  echo "LIVE-ORPHAN-COUNT: FAIL the new test is red after restore"
  exit 1
}
echo "GREEN $(grep -E '^test result' "$scratch/green.log" | head -1)"

# 10. The tree is byte-identical under the two source directories.
if [ -n "$(git status --porcelain crates/pii-server/src crates/pii-graph/src)" ]; then
  echo "LIVE-ORPHAN-COUNT: FAIL crates/pii-server/src or crates/pii-graph/src is not clean"
  exit 1
fi
echo "LIVE-ORPHAN-COUNT: OK"
exit 0
