#!/usr/bin/env bash
# Test: the commit's lane comes from the run that changed the tree, and it survives a resume at verify (spec 304). Two blocks of cycle.sh
# are run alone, with no cycle and no commit: `# lane-record:begin` … `# lane-record:end` (the dispatch side: a run that changed the tree
# records its lane on the row) and `# lane-trailer:begin` … `# lane-trailer:end` (the commit side: the recorded lane first, then the LAST
# `LANE-GRAPH:` line of the worker log, then nothing). State is a scratch file (PII_STATE), never the live queue.
set -uo pipefail
cd "$(dirname "$0")/../../.." || exit 2
REPO=$(pwd); PASS=0; FAIL=0
ok()  { echo "ok   $1"; PASS=$((PASS+1)); }
bad() { echo "FAIL $1"; FAIL=$((FAIL+1)); }
SCRATCH=$(mktemp -d "$REPO/_tmp/lane-from-state-XXXXXX") || exit 2
TRAILER=$(sed -n '/# lane-trailer:begin/,/# lane-trailer:end/p' scripts/autonomy/cycle.sh)
RECORD=$(sed -n '/# lane-record:begin/,/# lane-record:end/p' scripts/autonomy/cycle.sh)
ITEM=304-a-row
export PII_STATE="$SCRATCH/state.json"

# the state the cycle reads: one row for the item, carrying the given lane (or none)
state_with() { # [<lane>]
  if [ -n "${1:-}" ]; then printf '{"%s": {"status": "done", "lane": "%s"}}\n' "$ITEM" "$1" > "$PII_STATE"
  else printf '{"%s": {"status": "done"}}\n' "$ITEM" > "$PII_STATE"; fi
}
lane_on_row() { python3 scripts/autonomy/state.py get "$ITEM" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("lane",""))'; }
# the message the cycle would commit for a worker log: runs the marked trailer block with the cycle's SPEC, TITLE, ITEM and WLOG
message_for() { # <worker-log>
  ( SPEC=304-a-commits-lane TITLE="a resumed row keeps its lane" ITEM=$ITEM WLOG=$1; eval "$TRAILER"; printf '%s' "$COMMIT_MSG" )
}
# the dispatch side: runs the marked record block for the worker log of the run that just finished
record_for() { # <worker-log>
  ( ITEM=$ITEM WLOG=$1; eval "$RECORD" )
}

printf '== coder.exited: {"tree_changed":"yes"}\nLANE-GRAPH: spec\n' > "$SCRATCH/spec-changed.log"
printf '== coder.exited: {"tree_changed":"no"}\nLANE-GRAPH: coder\n' > "$SCRATCH/coder-noop.log"
printf '== coder.exited: {"tree_changed":"yes"}\nLANE-GRAPH: coder\n' > "$SCRATCH/coder-only.log"
printf '== 304 via pii\nLANE-GRAPH: coder\nLANE-GRAPH: spec\n' > "$SCRATCH/last-wins.log"
printf '== 304 via pii\nno lane line here\n' > "$SCRATCH/none.log"
: > "$SCRATCH/empty.log"

echo "== the resumed run (no dispatch, empty WLOG) takes the lane from the state"
state_with spec
M=$(message_for "$SCRATCH/empty.log")
if [ "$(printf '%s\n' "$M" | tail -1)" = "Lane: spec" ]; then ok "trailer_from_the_recorded_lane: the message ends with Lane: spec"
else bad "trailer_from_the_recorded_lane: message ends [$(printf '%s\n' "$M" | tail -1)]"; fi

echo "== no record: the log decides"
state_with
M=$(message_for "$SCRATCH/coder-only.log")
if [ "$(printf '%s\n' "$M" | tail -1)" = "Lane: coder" ]; then ok "the_log_wins_when_no_record_exists: no lane on the row, LANE-GRAPH: coder gives Lane: coder"
else bad "the_log_wins_when_no_record_exists: message ends [$(printf '%s\n' "$M" | tail -1)]"; fi

echo "== control: no record and no lane line, no trailer"
state_with
M=$(message_for "$SCRATCH/none.log")
if ! printf '%s\n' "$M" | grep -q '^Lane:'; then ok "CONTROL_no_record_no_log_no_trailer: no Lane: line"
else bad "CONTROL_no_record_no_log_no_trailer: got a Lane: line"; fi

echo "== the dispatch side records the lane of a run that changed the tree"
state_with
record_for "$SCRATCH/spec-changed.log"
if [ "$(lane_on_row)" = spec ]; then ok "the_dispatch_side_records_the_lane: the row holds lane=spec"
else bad "the_dispatch_side_records_the_lane: the row holds [$(lane_on_row)]"; fi

echo "== a run that changed nothing does not overwrite the record"
state_with
record_for "$SCRATCH/spec-changed.log"
record_for "$SCRATCH/coder-noop.log"
if [ "$(lane_on_row)" = spec ]; then ok "a_no_op_run_does_not_overwrite_the_record: the record stays spec"
else bad "a_no_op_run_does_not_overwrite_the_record: the record is [$(lane_on_row)]"; fi

echo "== the last LANE-GRAPH line wins"
state_with
M=$(message_for "$SCRATCH/last-wins.log")
if [ "$(printf '%s\n' "$M" | tail -1)" = "Lane: spec" ]; then ok "LAST_lane_line_wins: coder then spec gives Lane: spec"
else bad "LAST_lane_line_wins: message ends [$(printf '%s\n' "$M" | tail -1)]"; fi

mkdir -p "$REPO/_tmp/_trash" && mv "$SCRATCH" "$REPO/_tmp/_trash/lane-from-state-$(date +%s)"
echo "PASS: $PASS FAIL: $FAIL"
[ "$FAIL" = 0 ]
