#!/usr/bin/env bash
# pii-backup.sh — copy every live store to <root>/<UTC stamp>/, verify each copy against its source, then publish.
#
# A half-written backup must never look like a good one: copies go into <stamp>.partial/, every copy is re-hashed
# against its source, manifest.tsv is written, and only then is the directory renamed into place. A mismatch leaves
# the .partial directory behind and exits 1. Retention moves old backups to <root>/_trash/ — nothing is removed.
#
# Usage: pii-backup.sh [--source DIR] [--root DIR] [--keep DAYS] [--dry-run]
# Exit: 0 published; 1 a copy failed verification; 2 refused (missing source, no stores, bad usage).
set -uo pipefail

SOURCE=${PII_STATE_DIR:-$HOME/.local/state/pii}
ROOT=$HOME/.local/state/pii/backups
KEEP=14
DRY=0
while [ $# -gt 0 ]; do
  case $1 in
    --source) SOURCE=$2; shift 2 ;;
    --root) ROOT=$2; shift 2 ;;
    --keep) KEEP=$2; shift 2 ;;
    --dry-run) DRY=1; shift ;;
    *) echo "usage: pii-backup.sh [--source DIR] [--root DIR] [--keep DAYS] [--dry-run]" >&2; exit 2 ;;
  esac
done

# A missing source is a refusal, never an empty backup.
[ -d "$SOURCE" ] || { echo "refused: source $SOURCE is not a directory" >&2; exit 2; }
NAMES=$(find "$SOURCE" -maxdepth 1 -type f -name '*.redb' -printf '%f\n' | sort)
[ -n "$NAMES" ] || { echo "refused: no stores (*.redb) in $SOURCE" >&2; exit 2; }

STAMP=$(date -u +%Y%m%dT%H%M%SZ)
DEST=$ROOT/$STAMP
PART=$ROOT/$STAMP.partial

if [ "$DRY" -eq 1 ]; then
  echo "plan: source $SOURCE"
  total=0
  for name in $NAMES; do
    bytes=$(stat -c %s "$SOURCE/$name")
    total=$((total + bytes))
    echo "plan: $name $bytes bytes"
  done
  echo "plan: $(printf '%s\n' "$NAMES" | wc -l) stores, $total bytes"
  echo "plan: destination $DEST"
  exit 0
fi

[ -e "$DEST" ] && { echo "refused: $DEST already exists" >&2; exit 1; }
mkdir -p "$ROOT" && mkdir "$PART" || { echo "refused: cannot create $PART" >&2; exit 1; }

: > "$PART/manifest.tsv"
total=0
for name in $NAMES; do
  cp "$SOURCE/$name" "$PART/$name" || { echo "FAIL copy $name" >&2; exit 1; }
  # Test hook: shorten a copy before it is verified, so the refusal can be shown.
  if [ "${PII_BACKUP_SHORTEN:-}" = "$name" ]; then
    size=$(stat -c %s "$PART/$name")
    truncate -s $((size - 1)) "$PART/$name"
  fi
  src_sha=$(sha256sum "$SOURCE/$name" | cut -d' ' -f1)
  dst_sha=$(sha256sum "$PART/$name" | cut -d' ' -f1)
  bytes=$(stat -c %s "$PART/$name")
  if [ "$src_sha" != "$dst_sha" ] || [ "$bytes" != "$(stat -c %s "$SOURCE/$name")" ]; then
    echo "MISMATCH $name: copy does not match source; $PART left in place" >&2
    exit 1
  fi
  printf '%s\t%s\t%s\n' "$name" "$bytes" "$dst_sha" >> "$PART/manifest.tsv"
  printf '%s %s %s\n' "$name" "$bytes" "$dst_sha"
  total=$((total + bytes))
done

mv "$PART" "$DEST" || { echo "FAIL rename $PART" >&2; exit 1; }
echo "backup: $(printf '%s\n' "$NAMES" | wc -l) stores, $total bytes, verified"
echo "backup: $DEST"

# Retention: keep --keep days, never the newest. An old backup is moved to _trash/, never removed.
newest=$(find "$ROOT" -mindepth 1 -maxdepth 1 -type d -name '[0-9]*T[0-9]*Z' -printf '%f\n' | sort | tail -1)
cutoff=$(( $(date -u +%s) - KEEP * 86400 ))
for d in $(find "$ROOT" -mindepth 1 -maxdepth 1 -type d -name '[0-9]*T[0-9]*Z' -printf '%f\n' | sort); do
  [ "$d" = "$newest" ] && continue
  epoch=$(date -u -d "${d:0:4}-${d:4:2}-${d:6:2}T${d:9:2}:${d:11:2}:${d:13:2}Z" +%s)
  if [ "$epoch" -lt "$cutoff" ]; then
    mkdir -p "$ROOT/_trash" && mv "$ROOT/$d" "$ROOT/_trash/$d" && echo "retention: moved $d to _trash/"
  fi
done
exit 0
