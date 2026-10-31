#!/usr/bin/env bash
# run-tests.sh [DIR] — run every scripts/autonomy/tests/*.test.sh and say, per file, whether it passed (spec 324).
#
# Why this exists: measured 2026-10-31, `tick-message-worker-rc.test.sh` was red for two days. Commit 8b1cb08
# (spec 318) changed tick.sh's phone line and left the test on the old sentence, and nothing noticed because
# the autonomy shell tests are not part of `cargo test` — the gate cannot go red on a test it never runs.
# A shell test outside the gate is a comment with an exit code.
#
# A test that cannot run here DECLARES it, on its own first lines:
#     # requires: dev-tree
# The runner then prints SKIP with the reason instead of failing it — and a SKIP is never counted as a pass.
# An unknown requirement is REFUSED (exit 2), so a typo can never quietly skip a test.
#
# Usage: run-tests.sh [DIR]     DIR defaults to the directory this script lives in, plus /tests.
# Exit: 0 when every runnable file passed, 1 when one failed, 2 on bad usage / no tests / unknown requirement.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
DIR=${1:-$HERE/tests}
[ -d "$DIR" ] || { echo "run-tests: no such directory: $DIR" >&2; exit 2; }
ROOT=$(cd "$DIR/../../.." && pwd)   # tests -> autonomy -> scripts -> repo root
shopt -s nullglob
FILES=("$DIR"/*.test.sh)
shopt -u nullglob
[ "${#FILES[@]}" -gt 0 ] || { echo "run-tests: REFUSED — no *.test.sh in $DIR (a runner with nothing to run must not report success)" >&2; exit 2; }
# A non-interactive shell does not inherit the interactive PATH (repo rule: set the environment inside the
# script). Measured 2026-10-31: the gate ran this step with `uv: command not found` and 5 tests that use
# `uv run` failed, while the same tests passed by hand -- the tool dir was simply not on PATH there.
export PATH="/root/.local/bin:/usr/local/cargo/bin:$PATH"
MISSING_TOOLS=""
for tool in uv git python3; do
  command -v "$tool" >/dev/null 2>&1 || MISSING_TOOLS="$MISSING_TOOLS $tool"
done
[ -z "$MISSING_TOOLS" ] || echo "run-tests: WARNING missing tools:$MISSING_TOOLS — tests that need them will fail" >&2
PER_TEST_TIMEOUT=${AUTONOMY_TEST_TIMEOUT:-600}
PASS=0; FAIL=0; SKIP=0; RED=""; SKIPPED=""

# cap_ok <capability> — is it available for the tree under test?
cap_ok() {
  case "$1" in
    # the test clones the absolute dev repo, so it only means anything when the tree under test IS that repo
    dev-tree) [ -d "$ROOT/.git" ] ;;
    *) echo "run-tests: UNKNOWN requirement '$1' — add it to cap_ok() before a test may declare it" >&2; return 2 ;;
  esac
}

for T in "${FILES[@]}"; do
  NAME=$(basename "$T" .test.sh)
  # strict on purpose: the line must carry the capability list and nothing else. A malformed declaration is
  # refused by cap_ok (exit 2) rather than guessed at, so a stray word can never silently skip a test.
  REQ=$(sed -n '1,8s/^# requires: *//p' "$T" | head -1)
  MISSING=""
  if [ -n "$REQ" ]; then
    IFS=',' read -r -a CAPS <<< "$REQ"
    for C in "${CAPS[@]}"; do
      C=$(echo "$C" | tr -d ' ')
      cap_ok "$C"; RC=$?
      if [ "$RC" -eq 2 ]; then exit 2; fi
      [ "$RC" -eq 0 ] || MISSING="$MISSING $C"
    done
  fi
  if [ -n "$MISSING" ]; then
    SKIP=$((SKIP+1)); SKIPPED="$SKIPPED $NAME"
    echo "SKIP $NAME — requires$MISSING (not available under $ROOT); it is NOT counted as a pass"
    continue
  fi
  OUT=$(timeout "$PER_TEST_TIMEOUT" bash "$T" 2>&1)
  RC=$?
  TAIL=$(printf '%s\n' "$OUT" | tail -1)
  if [ "$RC" -eq 0 ]; then
    PASS=$((PASS+1)); echo "ok   $NAME — $TAIL"
  else
    FAIL=$((FAIL+1)); RED="$RED $NAME"
    echo "FAIL $NAME (rc=$RC) — $TAIL"
    # enough output to diagnose from the gate log without re-running by hand
    printf '%s\n' "$OUT" | tail -25 | sed 's/^/     | /'
  fi
done
echo "autonomy tests: $PASS passed, $FAIL failed, $SKIP skipped (dir: $DIR, root: $ROOT)"
[ -z "$SKIPPED" ] || echo "skipped:$SKIPPED"
[ "$FAIL" -eq 0 ] || { echo "red:$RED"; exit 1; }
