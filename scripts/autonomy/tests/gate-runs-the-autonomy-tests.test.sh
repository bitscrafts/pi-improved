#!/usr/bin/env bash
# gate-runs-the-autonomy-tests.test.sh — the gate must RUN the autonomy shell tests, and the runner must be
# able to fail (spec 324). Measured 2026-10-31: tick-message-worker-rc.test.sh was red for two days after
# 8b1cb08 changed tick.sh; the gate stayed green because it never ran these files.
# Runs the REAL runner against scratch directories; never runs a cycle, a worker or a gate.
set -u
cd "$(dirname "$0")/../../.." || exit 1
OK=0
FAILS=0
check(){
  if [ "$2" = "$3" ]; then echo "ok $1"; OK=$((OK+1)); else echo "FAIL $1"; echo "  got '$2', want '$3'" >&2; FAILS=$((FAILS+1)); fi
}
T=_tmp/gate-runs-autonomy-tests.$$
mkdir -p "$T/green" "$T/red"
trap 'find "$T" -depth -delete 2>/dev/null' EXIT
RUNNER=scripts/autonomy/run-tests.sh

# (a) the gate lists the step, in BOTH step lists, and the command is the runner
for LIST in gate-steps.txt gate-steps-quick.txt; do
  check "(a) $LIST has an autonomy step running the runner" \
    "$(grep -c "autonomy.*|.*bash scripts/autonomy/run-tests.sh" "$LIST")" "1"
done

# (b) a green directory passes
printf '#!/usr/bin/env bash\necho "2 ok, 0 FAIL"\nexit 0\n' > "$T/green/a.test.sh"
check "(b) a green directory exits 0" "$(bash $RUNNER "$T/green" >/dev/null 2>&1; echo $?)" "0"

# (c) THE POINT: a red test in the directory makes the runner exit 1 and name the file
printf '#!/usr/bin/env bash\necho "1 ok, 1 FAIL"\necho "  got X, want Y" >&2\nexit 1\n' > "$T/red/b.test.sh"
check "(c) a red directory exits 1" "$(bash $RUNNER "$T/red" >/dev/null 2>&1; echo $?)" "1"
check "(c) and the failure is named" "$(bash $RUNNER "$T/red" 2>&1 | grep -c 'FAIL b (rc=1)')" "1"
check "(c) and the failing test's own last line is kept" "$(bash $RUNNER "$T/red" 2>&1 | grep -c '1 ok, 1 FAIL')" "1"

# (d) CONTROL: a directory with no tests is refused, not reported as success
check "(d) CONTROL an empty directory exits 2" "$(bash $RUNNER "$T/empty-does-not-exist" >/dev/null 2>&1; echo $?)" "2"

# (e) the real directory it will run in the gate is not empty (a runner pointed at the wrong path must not pass)
check "(e) the real tests directory has files" "$([ "$(ls scripts/autonomy/tests/*.test.sh | wc -l)" -ge 20 ] && echo yes || echo no)" "yes"

# (f) a DECLARED requirement that is not available is SKIPPED, not failed and not counted as a pass
mkdir -p "$T/needs/scripts/autonomy/tests"
printf '#!/usr/bin/env bash\n# requires: dev-tree\necho "should never run"\nexit 1\n' > "$T/needs/scripts/autonomy/tests/needs-dev.test.sh"
OUT=$(bash $RUNNER "$T/needs/scripts/autonomy/tests" 2>&1); RC=$?
check "(f) an unavailable requirement exits 0" "$RC" "0"
check "(f) it is named SKIP, with the capability" "$(printf '%s' "$OUT" | grep -c 'SKIP needs-dev — requires dev-tree')" "1"
check "(f) and counted as skipped, not passed" "$(printf '%s' "$OUT" | grep -c '0 passed, 0 failed, 1 skipped')" "1"

# (g) the same declaration under a tree that HAS .git runs the test
mkdir -p "$T/gitroot/.git" "$T/gitroot/scripts/autonomy/tests"
printf '#!/usr/bin/env bash\n# requires: dev-tree\necho "ran in the dev tree"\nexit 0\n' > "$T/gitroot/scripts/autonomy/tests/needs-dev.test.sh"
check "(g) with .git present it runs and passes" "$(bash $RUNNER "$T/gitroot/scripts/autonomy/tests" 2>&1 | grep -c '1 passed, 0 failed, 0 skipped')" "1"

# (h) CONTROL: an unknown requirement is REFUSED, so a typo cannot quietly skip a test
mkdir -p "$T/typo/scripts/autonomy/tests"
printf '#!/usr/bin/env bash\n# requires: dev-treey\nexit 0\n' > "$T/typo/scripts/autonomy/tests/typo.test.sh"
check "(h) CONTROL an unknown requirement exits 2" "$(bash $RUNNER "$T/typo/scripts/autonomy/tests" >/dev/null 2>&1; echo $?)" "2"

# (i) the runner sets the tool PATH itself: a test that needs uv must find it even from a bare shell
mkdir -p "$T/tools/scripts/autonomy/tests"
printf '#!/usr/bin/env bash\ncommand -v uv >/dev/null || exit 1\necho "uv found"\n' > "$T/tools/scripts/autonomy/tests/needs-uv.test.sh"
check "(i) a test finds uv through the runner" "$(env -i PATH=/usr/bin:/bin HOME=/root bash $RUNNER "$T/tools/scripts/autonomy/tests" 2>&1 | grep -c '1 passed, 0 failed')" "1"

echo "$OK ok, $FAILS FAIL"
