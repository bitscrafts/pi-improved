#!/usr/bin/env bash
# Spec 197: the installer refuses without a deploy card for the revision it builds.
# Drives the REAL deploy/install-pii.sh as a subprocess, with scratch dirs under _tmp/.
#   --case=a  no card: non-zero exit, refusal text names the revision and both card paths
#   --case=b  control, card present: no card refusal in the output (a later abort is fine)
# No argument: both cases; non-zero if either fails.
set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
REV7="$(git -C "$REPO" rev-parse --short=7 HEAD)" || { echo "cannot read HEAD of $REPO" >&2; exit 2; }
REV12="$(git -C "$REPO" rev-parse --short=12 HEAD)"
mkdir -p "$REPO/_tmp"
SCRATCH="$(mktemp -d "$REPO/_tmp/installer-card-XXXXXX")" || exit 2

run_installer() {   # $1 = scratch case dir; prints combined output, returns the installer's rc
  SKIP_BUILD=1 PREFIX="$1/bin" CONFIG_DIR="$1/config" STATE_DIR="$1/state" \
    DEPLOY_CARD_DIR="$1/cards" LLAMA_HOST="http://127.0.0.1:1/v1" \
    bash "$REPO/deploy/install-pii.sh" 2>&1
}

case_a() {
  local d="$SCRATCH/a" out rc
  mkdir -p "$d/cards"
  out="$(run_installer "$d")"; rc=$?
  if [ "$rc" -ne 0 ] \
     && grep -qF "no deploy card for revision $REV7" <<<"$out" \
     && grep -qF "card-$REV7.json" <<<"$out" \
     && grep -qF "card-$REV12.json" <<<"$out"; then
    echo "case a: PASS (rc=$rc, refusal names $REV7 and both card files)"
  else
    echo "case a: FAIL (rc=$rc) output: $out"; return 1
  fi
}

case_b() {
  local d="$SCRATCH/b" out rc
  mkdir -p "$d/cards"
  echo '{"revision":"'"$REV7"'"}' > "$d/cards/card-$REV7.json"
  out="$(run_installer "$d")"; rc=$?
  if grep -qF "no deploy card for revision" <<<"$out"; then
    echo "case b: FAIL (rc=$rc) card present but refused: $out"; return 1
  fi
  echo "case b: PASS (rc=$rc, no card refusal with card-$REV7.json planted)"
}

status=0
case "${1:-}" in
  --case=a) case_a || status=1 ;;
  --case=b) case_b || status=1 ;;
  "")       case_a || status=1; case_b || status=1 ;;
  *) echo "usage: $0 [--case=a|--case=b]" >&2; status=2 ;;
esac
exit "$status"
