#!/usr/bin/env bash
# usage: build-release-for-host.sh [<rev>]   (default HEAD)
# Release build of pii for the host, from a CLEAN EXPORT of a COMMIT (never the working tree), stamped with that commit.
# Pass the revision the fresh-gate passed on: what ships must be what was gated, not whatever HEAD has become since.
# Same recipe as deploy/install-pii.sh step 1, kept as its own script so the build is reviewable and re-runnable
# (HARD RULE 0d) and its output is a directory the deploy script can checksum. Builds only `pii` (the host runs only it).
set -euo pipefail
export PATH=/usr/local/cargo/bin:/root/.local/bin:/usr/local/bin:/usr/bin:/bin:$PATH
unset PII_BUILD_REVISION CARGO_TARGET_DIR
cd /projects/pi-improved
REV12=$(git rev-parse --verify --short=12 "${1:-HEAD}^{commit}") || { echo "REFUSED: ${1:-HEAD} is not a commit"; exit 2; }
OUT=/projects/_tmp/release-$REV12
FREE_GB=$(df -BG /projects | tail -1 | tr -s ' ' | cut -d' ' -f4 | tr -d G)
[ "$FREE_GB" -ge 20 ] || { echo "REFUSED: only ${FREE_GB}G free on /projects (floor 20G)"; exit 3; }
echo "BUILD $REV12 start $(date -u +%FT%TZ) free=${FREE_GB}G out=$OUT"
EXPORT=$(mktemp -d /projects/_tmp/release-export-XXXXXX)
git archive "$REV12" | tar -x -C "$EXPORT"
export PII_BUILD_REVISION=$REV12
export CARGO_TARGET_DIR=/projects/_cache/targets/release-deploy
export TMPDIR=/projects/pi-improved/_tmp/tmpdir
mkdir -p "$CARGO_TARGET_DIR" "$OUT" "$TMPDIR"
(cd "$EXPORT" && cargo build --release -p pii-cli) 2>&1 | tail -4
BIN=$CARGO_TARGET_DIR/release/pii
[ -x "$BIN" ] || { echo "BUILD FAILED: no binary"; exit 1; }
cp "$BIN" "$OUT/pii"
echo "version : $("$OUT/pii" --version 2>&1 | head -1)"
LABEL=$(strings -n 12 "$OUT/pii" | grep -oE "${REV12}(-dirty)?" | sort -u | tr '\n' ' ')
echo "label   : $LABEL"
case "$LABEL" in *"$REV12 "*|"$REV12") ;; *) echo "BUILD FAILED: label is not exactly $REV12"; exit 1 ;; esac
case "$LABEL" in *dirty*) echo "BUILD FAILED: label says dirty"; exit 1 ;; esac
sha256sum "$OUT/pii" | tee "$OUT/pii.sha256"
stat -c 'size  : %s bytes' "$OUT/pii"
echo "export kept for the graph step: $EXPORT" | tee "$OUT/export-dir.txt"
echo "BUILD OK $REV12 end $(date -u +%FT%TZ)"
