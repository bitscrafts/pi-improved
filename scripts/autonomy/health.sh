#!/usr/bin/env bash
# health.sh — ONE screen answering "is the loop alive, and if not, since when and why?".
#
# WHY IT EXISTS (2026-10-15): a patch of ours made every cycle die on its first line —
#   `flock: failed to execute scripts/autonomy/cycle.sh: Permission denied` —
# and NOTHING reported it. The line sat in a log nobody reads, the Anthropic console froze at 7%, and the operator had
# to ask three times whether it was running. This is the reader that ends that class: a process cannot die silently if
# one command always tells you the last thing it said.
set -u
cd /projects/pi-improved || exit 2
S=_tmp/autonomy; L=$(ls -t $S/logs/cycle-*.log 2>/dev/null | head -1); now=$(date -u +%s)
printf '=== pii autonomy health  %s\n' "$(date -u +%H:%M:%SZ)"
printf 'control : %s\n' "$(tr -d '\n' < $S/control 2>/dev/null | cut -c1-70)"
printf 'workers : %s   cycles : %s\n' "$(pgrep -cf 'claude -p #')" "$(ps -eo cmd | grep -c 'autonomy/cycle')"
if [ -n "$L" ]; then
  printf 'cycle log : %s   idle %ss\n' "$(basename "$L")" "$(( now - $(stat -c %Y "$L") ))"
  printf 'outcome   : %s\n' "$(grep -a 'OUTCOME=' "$L" | tail -1)"
  printf 'last line : %s\n' "$(tail -1 "$L" | cut -c1-150)"
  printf 'last err  : %s\n' "$(grep -a -iE 'permission denied|not found|flock:|failed to execute' "$L" | tail -1 | cut -c1-150)"
fi
W=$(ls -t _tmp/claude-batchC/*.log 2>/dev/null | head -1)
[ -n "$W" ] && printf 'worker log: %s idle %ss size %sB\n' "$(basename "$W")" "$(( now - $(stat -c %Y "$W") ))" "$(stat -c %s "$W")"
printf 'queue     : %s\n' "$(python3 scripts/autonomy/state.py pick | cut -c1-44)"
printf 'account   : %s\n' "$(python3 scripts/autonomy/usage_json.py 2>/dev/null)"
printf 'host      : load %s  cpus %s\n' "$(cut -d' ' -f1-3 /proc/loadavg)" "$(nproc)"
printf 'git       : %s\n' "$(git log --oneline -1 | cut -c1-64)"
