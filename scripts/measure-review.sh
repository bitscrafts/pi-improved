#!/usr/bin/env bash
# measure-review.sh <rubric,rubric,...> <dir> — runs the live reviewers (graphs/review-one.toml) over every `.md` in <dir>
# and prints one row per document: its verdict under each rubric. Spec 403 R7.
#
# Corpus convention: `bad-*.md` must FAIL (at least one rubric refuses it) and `ok-*.md` must PASS (every rubric passes it).
# A reviewer that cannot run (exit 2 or 3, or any other status) is neither: it is printed as ERR and never counts as a pass.
# The script exits non-zero when any document breaks the convention, or when a reviewer could not run.
#
# This is a measurement, not a gate criterion: it needs a model. `<dir>` is repo-relative or under the repo, because the
# reviewer reads each document through the workspace. `REVIEW_MODEL` and `REVIEW_PROVIDER` name the inputs the review graph
# takes; the reviewer node itself runs on the daemon's default provider.
#
# Usage: measure-review.sh <rubric,rubric,...> <dir>
set -uo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
BIN=${PII:-$ROOT/target/debug/pii}
MODEL=${REVIEW_MODEL:-@planner}
PROVIDER=${REVIEW_PROVIDER:-default}
DB=$ROOT/_tmp/measure-review/store.redb

[ $# -eq 2 ] || { echo "usage: measure-review.sh <rubric,rubric,...> <dir>" >&2; exit 2; }
IFS=, read -ra RUBRICS <<< "$1"
DIR=${2%/}
[ -d "$DIR" ] || { echo "measure-review: no such directory: $DIR" >&2; exit 2; }
[ -x "$BIN" ] || { echo "measure-review: no pii binary at $BIN (cargo build -p pii-cli, or set PII)" >&2; exit 2; }
mkdir -p "$ROOT/_tmp/measure-review"

# The document as the graph names it: repo-relative.
case "$DIR" in
  "$ROOT"/*) REL=${DIR#"$ROOT"/} ;;
  /*) echo "measure-review: $DIR is outside the repository" >&2; exit 2 ;;
  *) REL=$DIR ;;
esac

stamp=$(date +%s)
broken=0
printf 'document\t%s\tconvention\n' "$(IFS=$'\t'; echo "${RUBRICS[*]}")"
for doc in "$DIR"/*.md; do
  [ -f "$doc" ] || continue
  name=$(basename "$doc")
  row="$name"
  any_fail=0; all_pass=1; any_err=0
  for rubric in "${RUBRICS[@]}"; do
    input=$(printf '{"doc":"%s/%s","rubric":"%s","workdir":"%s","model":"%s","provider":"%s"}' \
      "$REL" "$name" "$rubric" "$ROOT" "$MODEL" "$PROVIDER")
    run_id="review-$stamp-$$-$RANDOM"
    "$BIN" graph run "$ROOT/graphs/review-one.toml" --db "$DB" --run-id "$run_id" --input "$input" >/dev/null 2>&1
    code=$?
    case $code in
      0) verdict=PASS ;;
      1) verdict=FAIL; any_fail=1; all_pass=0 ;;
      *) verdict=ERR; any_err=1; all_pass=0 ;;
    esac
    row="$row"$'\t'"$verdict"
  done
  case $name in
    bad-*) if [ $any_fail -eq 1 ]; then conv=ok; else conv=BROKEN; broken=1; fi ;;
    ok-*)  if [ $all_pass -eq 1 ]; then conv=ok; else conv=BROKEN; broken=1; fi ;;
    *)     conv=unlabelled ;;
  esac
  [ $any_err -eq 1 ] && { conv="$conv+ERR"; broken=1; }
  printf '%s\t%s\n' "$row" "$conv"
done
exit $broken
