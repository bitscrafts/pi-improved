#!/usr/bin/env bash
# Test: every commit the cycle makes for a row carries its lane (spec 303). The commit message is built by the block cycle.sh marks with
# `# lane-trailer:begin` / `# lane-trailer:end`; this test runs that block alone on a worker log, so no commit is made. The trailer is the LAST
# `LANE-GRAPH:` line of the worker log, and there is no trailer when the log has none (an old log).
set -uo pipefail
cd "$(dirname "$0")/../../.." || exit 2
REPO=$(pwd); PASS=0; FAIL=0
ok()  { echo "ok   $1"; PASS=$((PASS+1)); }
bad() { echo "FAIL $1"; FAIL=$((FAIL+1)); }
SCRATCH=$(mktemp -d "$REPO/_tmp/lane-trailer-XXXXXX") || exit 2
BLOCK=$(sed -n '/# lane-trailer:begin/,/# lane-trailer:end/p' scripts/autonomy/cycle.sh)

# the message the cycle would commit for a log: runs the marked block with the cycle's SPEC, TITLE and WLOG
message_for() { # <worker-log>
  ( SPEC=303-a-resumed-queue-row TITLE="a resumed queue row keeps its lane" WLOG=$1; eval "$BLOCK"; printf '%s' "$COMMIT_MSG" )
}
printf '== 303 via pii\nLANE-GRAPH: spec\n' > "$SCRATCH/spec.log"
printf '== 303 via pii\nLANE-GRAPH: coder\nLANE-GRAPH: spec\n' > "$SCRATCH/last-wins.log"
printf '== 303 via pii\nLANE-GRAPH: spec\nLANE-GRAPH: coder\n' > "$SCRATCH/first-wins.log"
printf '== 303 via pii\nno lane line here\n' > "$SCRATCH/none.log"

echo "== the trailer is the lane of the log"
M=$(message_for "$SCRATCH/spec.log")
if [ "$(printf '%s\n' "$M" | tail -1)" = "Lane: spec" ]; then ok "commit_message_carries_the_lane: the message ends with Lane: spec"
else bad "commit_message_carries_the_lane: message ends [$(printf '%s\n' "$M" | tail -1)]"; fi
if printf '%s\n' "$M" | grep -q '^$' && printf '%s\n' "$M" | head -1 | grep -q 'automated pass'; then ok "the trailer sits in its own block after a blank line"
else bad "the trailer sits in its own block after a blank line"; fi

echo "== control: no lane line, no trailer"
M=$(message_for "$SCRATCH/none.log")
if ! printf '%s\n' "$M" | grep -q '^Lane:'; then ok "CONTROL_no_lane_line_no_trailer: no Lane: line"
else bad "CONTROL_no_lane_line_no_trailer: got a Lane: line"; fi

echo "== the last LANE-GRAPH line wins"
M=$(message_for "$SCRATCH/last-wins.log")
if [ "$(printf '%s\n' "$M" | tail -1)" = "Lane: spec" ]; then ok "last_lane_line_wins: coder then spec gives Lane: spec"
else bad "last_lane_line_wins: message ends [$(printf '%s\n' "$M" | tail -1)]"; fi
M=$(message_for "$SCRATCH/first-wins.log")
if [ "$(printf '%s\n' "$M" | tail -1)" = "Lane: coder" ]; then ok "first_lane_line_does_not_win: spec then coder gives Lane: coder"
else bad "first_lane_line_does_not_win: message ends [$(printf '%s\n' "$M" | tail -1)]"; fi

mkdir -p "$REPO/_tmp/_trash" && mv "$SCRATCH" "$REPO/_tmp/_trash/cycle-lane-trailer-$(date +%s)"
echo "PASS: $PASS FAIL: $FAIL"
[ "$FAIL" = 0 ]
