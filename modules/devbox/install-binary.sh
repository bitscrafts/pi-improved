#!/usr/bin/env bash
# modules/devbox/install-binary.sh <target_dir> <dest> — install <target_dir>/debug/pii at <dest>, only when the copy is byte-identical (spec 279).
# daemon.sh calls it inside the container with the container's real target dir, so the binary installed is the one the build just wrote.
# It touches only <dest>.new and <dest>, and removes only <dest>.new.
set -uo pipefail
if [ $# -ne 2 ]; then echo "usage: install-binary.sh <target_dir> <dest>" >&2; exit 2; fi
SRC=$1/debug/pii
DEST=$2
if [ ! -f "$SRC" ] || [ ! -x "$SRC" ]; then echo "install-binary: no executable $SRC" >&2; exit 1; fi
if ! cp "$SRC" "$DEST.new"; then rm -f "$DEST.new"; echo "install-binary: copy of $SRC failed" >&2; exit 1; fi
want=$(sha256sum "$SRC" | cut -d' ' -f1)
have=$(sha256sum "$DEST.new" | cut -d' ' -f1)
if [ "$want" != "$have" ]; then
  rm -f "$DEST.new"
  echo "install-binary: copy mismatch: source ${want:0:12}, copy ${have:0:12}; dest unchanged" >&2
  exit 1
fi
mv -f "$DEST.new" "$DEST" || { echo "install-binary: could not move the copy to $DEST" >&2; exit 1; }
echo "installed ${want:0:12} from $SRC"
