#!/usr/bin/env bash
# Test: scripts/autonomy/env.sh reads the lane graph from a one-word file (spec 295), so the cron tick can switch the queue without
# an edit. Every case sources the REAL env.sh in a fresh bash with PII_LANE_FILE pointed at a scratch file and prints what it left in
# PII_LANE_GRAPH. Controls: an absent file, a garbage word, an empty file and an environment value that must win over the file.
set -uo pipefail
cd "$(dirname "$0")/../../.." || exit 2
REPO=$(pwd); PASS=0; FAIL=0
ENV_SH=$REPO/scripts/autonomy/env.sh
ok()  { echo "ok   $1"; PASS=$((PASS+1)); }
bad() { echo "FAIL $1"; FAIL=$((FAIL+1)); }
SCRATCH=$(mktemp -d "$REPO/_tmp/lane-knob-XXXXXX") || exit 2
probe() { # <lane-file> [VAR=value ...] -> what env.sh leaves in PII_LANE_GRAPH ("<unset>" when nothing); any stray output shows up too
  local f=$1; shift
  env -u PII_LANE_GRAPH PII_LANE_FILE="$f" "$@" bash -c 'source "$1"; printf "%s" "${PII_LANE_GRAPH-<unset>}"' _ "$ENV_SH" 2>&1
}

printf 'spec\n' > "$SCRATCH/spec"
printf 'coder\n' > "$SCRATCH/coder"
printf 'spec  \r\n' > "$SCRATCH/padded"
printf 'rm -rf /\n' > "$SCRATCH/garbage"
: > "$SCRATCH/empty"

[ "$(probe "$SCRATCH/spec")" = spec ] && ok file_spec_exports_spec || bad "file_spec_exports_spec (got [$(probe "$SCRATCH/spec")])"
[ "$(probe "$SCRATCH/coder")" = coder ] && ok file_coder_exports_coder || bad "file_coder_exports_coder (got [$(probe "$SCRATCH/coder")])"
[ "$(probe "$SCRATCH/padded")" = spec ] && ok trailing_spaces_and_crlf_are_stripped || bad "trailing_spaces_and_crlf_are_stripped (got [$(probe "$SCRATCH/padded")])"
[ "$(probe "$SCRATCH/absent")" = "<unset>" ] && ok CONTROL_absent_file_leaves_it_unset || bad "CONTROL_absent_file_leaves_it_unset (got [$(probe "$SCRATCH/absent")])"
[ "$(probe "$SCRATCH/garbage")" = "<unset>" ] && ok CONTROL_garbage_word_leaves_it_unset || bad "CONTROL_garbage_word_leaves_it_unset (got [$(probe "$SCRATCH/garbage")])"
[ "$(probe "$SCRATCH/spec" PII_LANE_GRAPH=coder)" = coder ] && ok CONTROL_env_wins_over_file || bad "CONTROL_env_wins_over_file (got [$(probe "$SCRATCH/spec" PII_LANE_GRAPH=coder)])"
[ "$(probe "$SCRATCH/empty")" = "<unset>" ] && ok CONTROL_empty_file_leaves_it_unset || bad "CONTROL_empty_file_leaves_it_unset (got [$(probe "$SCRATCH/empty")])"

printf 'spec' > "$SCRATCH/nonl"   # `echo -n spec`: read returns 1 at EOF although the variable is set
G=$(env -u PII_LANE_GRAPH PII_LANE_FILE="$SCRATCH/nonl" bash -e -c 'source "$1"; printf "%s" "${PII_LANE_GRAPH-<unset>}"' _ "$ENV_SH" 2>&1)
[ "$G" = spec ] && ok no_trailing_newline_works_under_set_e || bad "no_trailing_newline_works_under_set_e (got [$G])"
# the default path is the one the cron tick reads: it must sit beside the loop's real control file, or the knob is a file nobody looks at
DEF=$(env -u PII_LANE_GRAPH -u PII_LANE_FILE bash -c 'source "$1"; printf "%s" "$PII_LANE_FILE"' _ "$ENV_SH" 2>&1)
if [ -n "$DEF" ] && [ -e "$(dirname "$DEF")/control" ] && [ "$(basename "$DEF")" = lane-graph ]; then ok "default_path_sits_beside_the_control_file ($DEF)"; else bad "default_path_sits_beside_the_control_file (got [$DEF])"; fi
mkdir -p "$REPO/_tmp/_trash" && mv "$SCRATCH" "$REPO/_tmp/_trash/lane-knob-$(date +%s)-$$"
# the knob must not change what the other lane tests see: the cycle runs a row's criteria with env.sh sourced, so a test that expects the default lane has to pin it itself
# (measured: lane-graph-select 7 ok 1 FAIL and lane-graph-wall 0 ok 3 FAIL with PII_LANE_GRAPH=spec exported)
for t in lane-graph-select lane-graph-wall; do
  R=$(PII_LANE_GRAPH=spec bash "$REPO/scripts/autonomy/tests/$t.test.sh" 2>&1 | tail -1)
  case "$R" in *"FAIL: 0") ok "lane_tests_survive_the_knob_set_to_spec: $t";; *) bad "lane_tests_survive_the_knob_set_to_spec: $t ($R)";; esac
done
echo "PASS: $PASS FAIL: $FAIL"
