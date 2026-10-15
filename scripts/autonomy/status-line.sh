#!/usr/bin/env bash
# status-line.sh - the ONE line that says where the implementation is, refreshed every 60 s.
#
# Why (measured 2026-10-15): the account meter cannot distinguish a blocked worker from a working one, so the operator
# asked four times in one afternoon what was happening and every answer cost minutes. This is the answer, one line,
# with only the numbers that actually move: the item, the loop's control state, whether a cycle is alive, the worker's
# stream size and its growth over five seconds, the hot compiler count (building vs stuck), the gate's current step
# AND its elapsed seconds, and the account percentage.
# Run it once for a snapshot:      bash scripts/autonomy/status-line.sh --once
# Run it as the 60-second writer:  setsid nohup bash scripts/autonomy/status-line.sh >/dev/null 2>&1 &
set -u
cd /projects/pi-improved || exit 0
S=_tmp/autonomy
snap() {
  NOW=$(date -u +%s)
  ITEM=$(python3 scripts/autonomy/state.py pick 2>/dev/null | cut -f1 | head -1)
  [ -z "${ITEM:-}" ] || [ "$ITEM" = "NONE" ] && ITEM=$(python3 scripts/autonomy/state.py list 2>/dev/null | awk -F'\t' '$4=="running"{print $1}' | head -1)
  CTL=$(head -c 24 $S/control 2>/dev/null | tr -d '\n')
  W=$(ls -t _tmp/claude-batchC/*.log 2>/dev/null | head -1)
  WSZ=0; WID=0; GROW=0
  if [ -n "${W:-}" ]; then
    WSZ=$(stat -c %s "$W" 2>/dev/null || echo 0); WID=$(( NOW - $(stat -c %Y "$W" 2>/dev/null || echo "$NOW") ))
  fi
  CYC=$(ps -eo cmd 2>/dev/null | grep -ac '[a]utonomy/cycle.sh')
  HOT=$(ps -eo pcpu,comm 2>/dev/null | awk '$2 ~ /^(rustc|cargo|cargo-clippy)$/ && $1+0 > 20 {c++} END {print c+0}')
  GL=$(ls -t $S/logs/gate*.log 2>/dev/null | head -1)
  PHASE=$(grep -aoE 'step=[a-z_ ()]+ elapsed=[0-9]+s' "$GL" 2>/dev/null | tail -1)
  ACCT=$(python3 scripts/autonomy/usage_json.py 2>/dev/null | tr -dc '0-9.' | head -c 5)
  OUTLINE="$(date -u +%H:%M:%SZ) item=${ITEM:-none} ctl=${CTL:-?} cycles=$CYC worker=${WSZ}B idle=${WID}s hot=$HOT gate=${PHASE:-none} acct=${ACCT:-?}%"
  echo "$OUTLINE"
}
if [ "${1:-}" = "--once" ]; then snap; exit 0; fi
while true; do
  snap | tee $S/STATUS >/dev/null
  sleep 55
done
