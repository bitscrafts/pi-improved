#!/usr/bin/env bash
# lane-scripts-find-their-root.test.sh — the lane script and its tests work from a clone at any path (spec 411).
#
# The WORKING tree (every tracked file plus every new file that is not ignored, as on disk: a worker never commits, so the change must
# be tested before it is committed) is copied to a scratch directory under _tmp/, and the three lane tests run there with
# the gate's environment: lane-graph-wall, lane-preflight and lane-graph-knob must all pass. Controls: a copy of the lane script
# with the root hard-coded again must make lane-graph-wall fail in the copy, and a copy whose queue switch moved must make the
# knob's default-path case fail. The real repository is only read, except for the graphs the real-path check and the hard-coded
# control leave under its _tmp/devbox-test/graphs; those are moved aside to _tmp/_trash, never deleted.
#
# Exit: 0 when every check passes. The last line is `PASS: n FAIL: m`.
set -uo pipefail
REAL=$(cd "$(dirname "$0")/../../.." && pwd)
cd "$REAL" || exit 2
PASS=0; FAIL=0
ok()  { echo "ok   $1"; PASS=$((PASS+1)); }
bad() { echo "FAIL $1"; FAIL=$((FAIL+1)); }
# the gate's environment: env.sh is sourced, so PII_ROOT is the REAL repository (set below); the lane knob and the other queue variables are cleared; plus the usual tool path
unset PII_LANE_ROOT PII_LANE_GRAPH PII_LANE_FILE PII_CONTROL PII_LOCK PII_STATE PII_PLAN PII_SPECS PII_DRAIN_ROOT PII_CYCLE_LOCKED PII_FORCE_ITEM
export PII_ROOT="$REAL"   # the gate runs every test with env.sh sourced, so PII_ROOT is the REAL repository: a copy must not take its root from it
export PATH="/root/.local/bin:/usr/local/cargo/bin:$PATH"
# uv keeps its cache and its interpreters under the checkout, as lane-preflight does: the lane-graph-wall cases run uv and must not depend on a writable /root
export UV_CACHE_DIR=${UV_CACHE_DIR:-$REAL/_tmp/uv-cache} UV_PYTHON_INSTALL_DIR=${UV_PYTHON_INSTALL_DIR:-$REAL/_tmp/uv-python}

mkdir -p "$REAL/_tmp"
T=$(mktemp -d "$REAL/_tmp/lane-root-XXXXXX") || exit 2
trap 'mkdir -p "$REAL/_tmp/_trash" && mv "$T" "$REAL/_tmp/_trash/lane-root-$$-$(date +%s%N)" 2>/dev/null' EXIT

# export_tree <dir>: the working tree as on disk, with an empty _tmp/ for the tests to write into. <dir> is new under $T (mktemp), so nothing is removed first.
export_tree() {
  [ ! -e "$1" ] && mkdir -p "$1/_tmp" && git -C "$REAL" ls-files -z --cached --others --exclude-standard | tar --null -c -f - -C "$REAL" -T - | tar -x -C "$1"
}
# run_test <dir> <name> <tag>: runs one lane test from inside <dir>; its output goes to $T/<name>.<tag>.log
run_test() {
  # a clone's cycle exports PII_ROOT = that clone (env.sh), so the copy gets its own, never the real repository's
  (cd "$1" && PII_ROOT="$(pwd)" bash "scripts/autonomy/tests/$2.test.sh") >"$T/$2.$3.log" 2>&1
}
# verdict_ok <log>: the test's own last line reports no failure (lane-graph-wall exits 0 even when it fails)
verdict_ok() {
  tail -1 "$1" | grep -qE 'FAIL: 0$'
}

# the real repository's graphs, so a control that runs the script at the real path can be undone afterwards
GR=$REAL/_tmp/devbox-test/graphs
mkdir -p "$GR"
graph_list() { ls "$GR" 2>/dev/null | sort; }
trash_new_graphs() { # <before-file>: moves each graph that is not in the before-list to _trash
  graph_list | comm -13 "$1" - | while read -r f; do
    mkdir -p "$REAL/_tmp/_trash"; mv "$GR/$f" "$REAL/_tmp/_trash/lane-root-$(date +%s%N)-$f"
  done
}

# 1. the three lane tests pass in an export of the committed tree
TREE=$T/tree
export_tree "$TREE" || { echo "lane-scripts-find-their-root: REFUSED — the working tree could not be copied" >&2; exit 2; }
for t in lane-graph-wall lane-preflight lane-graph-knob; do
  if run_test "$TREE" "$t" tree && verdict_ok "$T/$t.tree.log"; then ok "the_three_lane_tests_pass_in_a_copy_of_the_tree: $t"
  else bad "the_three_lane_tests_pass_in_a_copy_of_the_tree: $t ($(tail -1 "$T/$t.tree.log"))"; fi
done

# 2. CONTROL: the lane script with the root hard-coded again makes lane-graph-wall fail in the copy
CTRL=$T/control-root
export_tree "$CTRL" || exit 2
sed -i 's|^REPO=.*|REPO=/projects/pi-improved|' "$CTRL/scripts/run-batchC-pii.sh"
graph_list > "$T/graphs-before-control.txt"
run_test "$CTRL" lane-graph-wall control
trash_new_graphs "$T/graphs-before-control.txt"
if ! verdict_ok "$T/lane-graph-wall.control.log" && grep -q 'the script left no graph' "$T/lane-graph-wall.control.log"; then
  ok "CONTROL_a_hard_coded_root_fails_in_the_copy"
else
  bad "CONTROL_a_hard_coded_root_fails_in_the_copy ($(tail -1 "$T/lane-graph-wall.control.log"))"
fi

# 3. the lane script keeps its values when it runs from the real path
graph_list > "$T/graphs-before-real.txt"
OUT=$(cd "$REAL" && ALLOW_DIRTY=1 PII_SSH=/bin/false bash scripts/run-batchC-pii.sh 286 --dry-run 2>&1)
trash_new_graphs "$T/graphs-before-real.txt"
LOGV=$(printf '%s\n' "$OUT" | sed -n 's/^LOG *: //p' | head -1)
GRAPHV=$(printf '%s\n' "$OUT" | sed -n 's/^\(GRAPH\|INPUT\) *: //p' | head -1)
case "$LOGV" in /projects/pi-improved/_tmp/claude-batchC/*) LOGOK=1 ;; *) LOGOK=0 ;; esac
case "$GRAPHV" in /projects/pi-improved/_tmp/devbox-test/graphs/*) GRAPHOK=1 ;; *) GRAPHOK=0 ;; esac
if [ "$LOGOK" = 1 ] && [ "$GRAPHOK" = 1 ]; then ok "the_lane_script_keeps_its_values_at_the_real_path"
else bad "the_lane_script_keeps_its_values_at_the_real_path (LOG [$LOGV] GRAPH [$GRAPHV])"; fi

# 4. the container path, used inside the devbox, is not the checkout root and stays as it is
if grep -q 'podman exec -w /projects/pi-improved pii-devbox' scripts/run-batchC-pii.sh; then
  ok "the_container_mount_path_is_unchanged"
else
  bad "the_container_mount_path_is_unchanged"
fi

# 5. the knob's default-path case passes in the export, which has no _tmp/autonomy/control file
if [ ! -e "$TREE/_tmp/autonomy/control" ] && grep -q '^ok   default_path_sits_beside_the_control_file' "$T/lane-graph-knob.tree.log"; then
  ok "the_knob_default_is_compared_by_directory"
else
  bad "the_knob_default_is_compared_by_directory ($(grep 'default_path_sits' "$T/lane-graph-knob.tree.log" | head -1))"
fi

# 6. CONTROL: when control.sh names another switch, the knob's default-path case fails
MOVED=$T/control-moved
export_tree "$MOVED" || exit 2
sed -i 's|ST=_tmp/autonomy;|ST=_tmp/moved-switch;|' "$MOVED/scripts/autonomy/control.sh"
run_test "$MOVED" lane-graph-knob moved
if grep -q '^FAIL default_path_sits_beside_the_control_file' "$T/lane-graph-knob.moved.log"; then
  ok "CONTROL_a_moved_switch_fails_the_knob_case"
else
  bad "CONTROL_a_moved_switch_fails_the_knob_case"
fi

echo "PASS: $PASS FAIL: $FAIL"
[ "$FAIL" -eq 0 ]
