#!/usr/bin/env bash
# pii-restore-drill.sh — restore the NEWEST backup into a scratch directory and prove it.
#
# Proof is two things: every restored file matches the manifest's sha256, and a daemon started on the scratch dir
# answers `pii health` with the same store names. It never touches the live stores: it reads the backup and writes
# only under its scratch dir. It does not restore over anything; an operator decides what to do with the result.
#
# Usage: pii-restore-drill.sh [--root DIR] [--scratch DIR]
# Exit: 0 both proofs hold; 1 a file mismatches or the daemon does not answer with the stores; 2 no backup.
set -uo pipefail

PII_BIN=${PII_BIN:-pii}
ROOT=$HOME/.local/state/pii/backups
SCRATCH=
while [ $# -gt 0 ]; do
  case $1 in
    --root) ROOT=$2; shift 2 ;;
    --scratch) SCRATCH=$2; shift 2 ;;
    *) echo "usage: pii-restore-drill.sh [--root DIR] [--scratch DIR]" >&2; exit 2 ;;
  esac
done

NEWEST=$(find "$ROOT" -mindepth 1 -maxdepth 1 -type d -name '[0-9]*T[0-9]*Z' -printf '%f\n' | sort | tail -1)
[ -n "$NEWEST" ] || { echo "refused: no backup under $ROOT" >&2; exit 2; }
SRC=$ROOT/$NEWEST
[ -n "$SCRATCH" ] || SCRATCH=$(mktemp -d "${TMPDIR:-/tmp}/pii-drill-XXXXXX")
mkdir -p "$SCRATCH" || exit 2
echo "drill: restoring $SRC into $SCRATCH"

# 1. restore, then every file matches the manifest
fail=0
while IFS=$'\t' read -r rel bytes sha; do
  cp "$SRC/$rel" "$SCRATCH/$rel" || { echo "MISMATCH $rel: cannot restore" >&2; fail=1; continue; }
  got=$(sha256sum "$SCRATCH/$rel" | cut -d' ' -f1)
  if [ "$got" = "$sha" ]; then
    echo "match $rel $bytes $sha"
  else
    echo "MISMATCH $rel: sha256 $got, manifest $sha" >&2
    fail=1
  fi
done < "$SRC/manifest.tsv"
[ "$fail" -eq 0 ] || { echo "DRILL: FAILED — restored files do not match the manifest" >&2; exit 1; }

# 2. a daemon on the scratch dir answers health with the same store names
# A stub model: the daemon refuses to start without one, and the drill must not reach a real provider.
printf '%s' '{"turns": [{"text": "stub", "tool_calls": [], "tokens_in": 1, "tokens_out": 1}]}' > "$SCRATCH/stub-model.json"
env PII_RUNTIME_DIR="$SCRATCH" PII_STATE_DIR="$SCRATCH" PII_CONFIG="$SCRATCH/pii.toml" PII_CONNECT_TIMEOUT_MS=500 \
  PII_MODEL="scripted:$SCRATCH/stub-model.json" "$PII_BIN" daemon --sandbox off --workspace "$SCRATCH" --log-file "$SCRATCH/drill-daemon.log" >/dev/null 2>&1 &
PID=$!
trap 'kill "$PID" 2>/dev/null; wait "$PID" 2>/dev/null' EXIT
HEALTH=
for _ in $(seq 60); do
  HEALTH=$(env PII_RUNTIME_DIR="$SCRATCH" PII_STATE_DIR="$SCRATCH" PII_CONNECT_TIMEOUT_MS=500 "$PII_BIN" health 2>&1) && break
  sleep 0.5
done
if ! printf '%s' "$HEALTH" | grep -q '^stores:'; then
  echo "DRILL: FAILED — the scratch daemon did not answer pii health" >&2
  exit 1
fi
while IFS=$'\t' read -r rel _ _; do
  if ! printf '%s' "$HEALTH" | grep -q "/$rel ("; then
    echo "MISMATCH $rel: store not listed by pii health" >&2
    exit 1
  fi
done < "$SRC/manifest.tsv"
echo "DRILL: ok — $(wc -l < "$SRC/manifest.tsv") stores restored, verified, and listed by pii health on $SCRATCH"
exit 0
