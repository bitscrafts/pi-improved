#!/usr/bin/env bash
# stall-check.sh — print ONE line when an implementation is stuck; print NOTHING when it is fine.
#
# Why it exists (2026-10-15): a 156 worker sat 19 minutes with its own cargo child at 6% CPU because a hand-run gate
# held cargo's artifact lock. The console showed "no activity" and no instrument said why.
#
# THE CONTROL (learned the same day, at the cost of a false alarm): a machine with a HOT rustc is BUILDING, not
# stalled. The first version of this script fired while eight rustc processes ran at 91-99% CPU. So the rule is:
# any compiler above 20% CPU means say nothing at all; only a cold, long-lived cargo/clippy qualifies as a stall.
set -u
cd /projects/pi-improved || exit 0
NOW=$(date -u +%s)

# 0. CONTROL: something is compiling → healthy by definition, print nothing
hot=$(ps -eo pcpu,comm 2>/dev/null | awk '$2 ~ /^(rustc|cargo|cargo-clippy)$/ && $1+0 > 20 {c++} END {print c+0}')
[ "${hot:-0}" -gt 0 ] && exit 0

alert=""

# 1. a worker in flight whose stream has stopped growing for 6+ minutes
W=$(ls -t _tmp/claude-batchC/*.log 2>/dev/null | head -1)
if [ -n "$W" ] && pgrep -f 'claude -p #' >/dev/null 2>&1; then
  widle=$(( NOW - $(stat -c %Y "$W") ))
  [ "$widle" -ge 360 ] && alert="STALL: worker $(basename "$W") stream idle ${widle}s ($(stat -c %s "$W")B)"
fi

# 2. a cargo/clippy alive >=10 min at low CPU (and sanity-bounded: a bogus etimes must not become an alarm)
eld=$(ps -eo etimes,pcpu,comm 2>/dev/null | awk '$3 ~ /^(cargo|cargo-clippy|rustc)$/ && $2+0 < 5.0 && $1+0 > 0 && $1+0 < 604800 {print $1}' | sort -rn | head -1)
if [ -z "$alert" ] && [ -n "${eld:-}" ] && [ "$eld" -ge 600 ]; then
  alert="STALL: a cargo/clippy step has been alive ${eld}s at under 5% CPU with no hot compiler - artifact-lock stall?"
fi

if [ -n "$alert" ]; then
  echo "$alert"
  echo "  compilers: $(ps -eo etimes,pcpu,comm 2>/dev/null | awk '$3 ~ /^(cargo|cargo-clippy|rustc)$/ {printf "%s/%ss/%s%% ", $3, $1, $2}' | cut -c1-150)"
  echo "  loop: $(head -c 60 _tmp/autonomy/control | tr '\n' ' ')"
  echo "  next: bash scripts/autonomy/health.sh"
fi
exit 0
