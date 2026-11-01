#!/usr/bin/env bash
# brief-evidence.sh <cycle-log> — the lines a corrective brief quotes back to a worker.
#
# Why this exists: cycle.sh built the resume brief with
#     grep -E 'failed steps:|failing tests:|panicked at|FAIL: |^OUTCOME=' "$LASTCYCLE"
# and the line that actually carries a failed verify is shaped
#     13 verify - 37600+37022 pid=- image=- crit=6/9 fail=bash scripts/autonomy/run-tests.sh ... rc=-
# so the pattern matched NOTHING, the brief shipped with an empty evidence block, and the worker was told
# "your earlier pass did not survive" three times running without ever being told what failed.
# Measured 2026-11-01 on row 330: its own report says "the brief's evidence block is empty again, so I still
# cannot see what worker-rc=1 was", the same mechanical failure repeated three times, and the row parked.
#
# Two rules, both learned there:
#   1. Match the shapes the cycle really writes — the verify/coder lines, run.refused, and the gate's own
#      patterns — not a hand-written subset of them.
#   2. Never print an empty block. A brief that says "Evidence:" and then nothing is worse than no brief:
#      it spends a paid worker on a retry. Fall back to the log's tail, which always exists.
#
# Read-only; no arguments beyond the log path. Exit 0 whether or not anything matched.
set -u
L=${1:-}
if [ -z "$L" ] || [ ! -f "$L" ]; then
  echo "(no cycle log to quote at '$L' — read the worker's own log under _tmp/claude-batchC/)"
  exit 0
fi
# `fail=` followed by a LETTER is a criterion command that failed; a green verify line ends `fail=- rc=0`.
# The alternative that matched every verify line was dropped on purpose: quoting a green line as "evidence"
# is the same lie in the other direction (control case 5).
PAT='failed steps:|failing tests:|panicked at|FAIL: |^OUTCOME=|crit=[0-9]+/[0-9]+ .*fail=[a-zA-Z]|run\.refused|^== why the run was not accepted|^== r[0-9]+ final'
EV=$(grep -E "$PAT" "$L" 2>/dev/null | head -20)
[ -n "$EV" ] || EV=$(tail -20 "$L")
printf '%s\n' "$EV"
