#!/usr/bin/env bash
# The backup, the restore drill and retention, tested against a real daemon — with a control for each refusal,
# because a backup that always says ok certifies a store nobody copied.
set -uo pipefail
cd "$(dirname "$0")/../../.." || exit 1
REPO=$PWD
B=scripts/checks/pii-backup.sh
D=scripts/checks/pii-restore-drill.sh
export PII_BIN=${PII_BIN:-$REPO/target/debug/pii}
ok=0; bad=0
pass(){ ok=$((ok+1)); echo "ok   $1"; }
fail(){ bad=$((bad+1)); echo "FAIL $1"; }
W=$(mktemp -d "${TMPDIR:-$REPO/_tmp}/pii-backup-XXXXXX") || exit 1
trap 'find "$W" -delete 2>/dev/null' EXIT

STORES="store memory channel timer"
junk_state(){ # junk_state <dir>: four store files of random bytes; the backup copies bytes, it does not parse them
  mkdir -p "$1"
  for f in $STORES; do head -c 200000 /dev/urandom > "$1/$f.redb"; done
}
real_state(){ # real_state <dir>: a fresh daemon creates real stores, answers health, and is stopped
  local dir=$1 pid
  mkdir -p "$dir"
  # A stub model: the daemon refuses to start without one, and the stores need no model to exist.
  printf '%s' '{"turns": [{"text": "stub", "tool_calls": [], "tokens_in": 1, "tokens_out": 1}]}' > "$dir.model.json"
  env PII_RUNTIME_DIR="$dir" PII_STATE_DIR="$dir" PII_CONFIG="$dir/pii.toml" PII_CONNECT_TIMEOUT_MS=500 \
    PII_MODEL="scripted:$dir.model.json" "$PII_BIN" daemon --sandbox off --workspace "$dir" --log-file "$dir/daemon.log" >/dev/null 2>&1 &
  pid=$!
  for _ in $(seq 60); do
    env PII_RUNTIME_DIR="$dir" PII_STATE_DIR="$dir" PII_CONNECT_TIMEOUT_MS=500 "$PII_BIN" health >/dev/null 2>&1 && break
    sleep 0.5
  done
  env PII_RUNTIME_DIR="$dir" PII_STATE_DIR="$dir" PII_CONNECT_TIMEOUT_MS=500 "$PII_BIN" health >/dev/null 2>&1
  local rc=$?
  kill "$pid" 2>/dev/null; wait "$pid" 2>/dev/null
  return $rc
}
stamped(){ find "$1" -mindepth 1 -maxdepth 1 -type d -name '[0-9]*T[0-9]*Z' | sort; }
manifest_matches(){ # manifest_matches <backup-dir> <source-dir>: every manifest row's sha256 is the source's
  local n=0 rel bytes sha
  while IFS=$'\t' read -r rel bytes sha; do
    n=$((n+1))
    [ "$(sha256sum "$2/$rel" | cut -d' ' -f1)" = "$sha" ] || return 1
  done < "$1/manifest.tsv"
  [ "$n" -gt 0 ]
}

# 1. every store copied, every hash equal to the source's
SRC=$W/src; junk_state "$SRC"
out=$(bash "$B" --source "$SRC" --root "$W/root1" 2>&1); rc=$?
dir=$(stamped "$W/root1" | head -1)
rows=$(wc -l < "$dir/manifest.tsv" 2>/dev/null || echo 0)
if [ "$rc" -eq 0 ] && [ -n "$dir" ] && [ "$rows" -eq 4 ] && manifest_matches "$dir" "$SRC"; then
  pass "backup_copies_every_store_with_a_matching_hash"
else
  fail "backup_copies_every_store_with_a_matching_hash (rc=$rc rows=$rows dir=$dir) $(printf '%s' "$out" | tail -2)"
fi

# 2. a control: a copy shortened before verification is refused, and leaves no good-looking backup behind
out=$(PII_BACKUP_SHORTEN=store.redb bash "$B" --source "$SRC" --root "$W/root2" 2>&1); rc=$?
if [ "$rc" -ne 0 ] && [ -n "$(find "$W/root2" -maxdepth 1 -type d -name '*.partial')" ] \
   && [ -z "$(stamped "$W/root2")" ]; then
  pass "CONTROL_a_truncated_copy_is_refused"
else
  fail "CONTROL_a_truncated_copy_is_refused (rc=$rc)"
fi

# 3. a missing source is a refusal, and nothing is created
out=$(bash "$B" --source "$W/nonexistent" --root "$W/root3" 2>&1); rc=$?
if [ "$rc" -eq 2 ] && [ ! -e "$W/root3" ] && [ ! -e "$W/nonexistent" ]; then
  pass "a_missing_source_is_a_refusal_not_an_empty_backup"
else
  fail "a_missing_source_is_a_refusal_not_an_empty_backup (rc=$rc)"
fi

# 4. dry run prints the plan and writes nothing
out=$(bash "$B" --source "$SRC" --root "$W/root4" --dry-run 2>&1); rc=$?
if [ "$rc" -eq 0 ] && [ ! -e "$W/root4" ] && printf '%s' "$out" | grep -q 'store.redb'; then
  pass "dry_run_writes_nothing"
else
  fail "dry_run_writes_nothing (rc=$rc)"
fi

# 5. a real backup of a real daemon's state restores into scratch and answers health with the same stores
real_state "$W/live" || fail "the_drill_restores_and_verifies (could not start a daemon on real stores)"
bash "$B" --source "$W/live" --root "$W/root5" >/dev/null 2>&1
out=$(bash "$D" --root "$W/root5" --scratch "$W/scratch5" 2>&1); rc=$?
dir=$(stamped "$W/root5" | tail -1)
# The drill hashes each restored file before its daemon starts (the daemon then writes into scratch), so the
# proof is the per-file "match" lines, one per manifest row, carrying the manifest's own sha256.
matched=0
while IFS=$'\t' read -r rel bytes sha; do
  printf '%s' "$out" | grep -q "^match $rel $bytes $sha$" && matched=$((matched+1))
done < "$dir/manifest.tsv"
if [ "$rc" -eq 0 ] && [ "$matched" -gt 0 ] && [ "$matched" -eq "$(wc -l < "$dir/manifest.tsv")" ]; then
  pass "the_drill_restores_and_verifies"
else
  fail "the_drill_restores_and_verifies (rc=$rc) $(printf '%s' "$out" | tail -2)"
fi

# 6. a control: one flipped byte in the backup fails the drill, and the drill names the file
cp -R "$W/root5" "$W/root6"
dir=$(stamped "$W/root6" | tail -1)
f=$dir/store.redb
b=$(dd if="$f" bs=1 skip=100 count=1 2>/dev/null | od -An -tu1 | tr -d ' ')
printf "\\x$(printf %02x $(( (b + 1) % 256 )))" | dd of="$f" bs=1 seek=100 count=1 conv=notrunc 2>/dev/null
out=$(bash "$D" --root "$W/root6" --scratch "$W/scratch6" 2>&1); rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'store.redb'; then
  pass "CONTROL_a_corrupt_backup_fails_the_drill"
else
  fail "CONTROL_a_corrupt_backup_fails_the_drill (rc=$rc)"
fi

# 7. retention moves an old backup to _trash/ and keeps the newest
mkdir -p "$W/root7/20200101T000000Z"
echo old > "$W/root7/20200101T000000Z/store.redb"
bash "$B" --source "$SRC" --root "$W/root7" >/dev/null 2>&1
if [ -d "$W/root7/_trash/20200101T000000Z" ] && [ ! -e "$W/root7/20200101T000000Z" ] \
   && [ -n "$(stamped "$W/root7")" ]; then
  pass "retention_moves_an_old_backup_it_never_deletes"
else
  fail "retention_moves_an_old_backup_it_never_deletes"
fi

echo "PII-BACKUP: $ok passed, $bad failed"
[ "$bad" -eq 0 ]
