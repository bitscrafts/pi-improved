#!/usr/bin/env bash
# queue-alarm.sh — THE thing that was missing on 2026-10-19: an alarm when the queue is idle with items in it.
#
# User, verbatim: "if i dont ask we would be hours without knowing nothing about the queue!"
#
# The rule this enforces: a queue with a dispatchable row and NO worker alive is a FAULT, not a state.
# Silence means healthy. Output (one line) means act. Empty stdout sends nothing (cron watchdog pattern).
#
# Conditions that are NOT faults, and must stay silent:
#   · control file not RUNNING            (an explicit stop)
#   · pick returns nothing                (queue genuinely empty)
#   · every open row parked/skipped       (named stop, spec 169)
#   · a worker or a cycle is alive        (work in flight)
#
# Env: PII_IDLE_MIN (default 12) — how long without a worker before it counts as idle.
set -uo pipefail
export PATH="/root/.local/bin:/usr/local/cargo/bin:/usr/local/bin:/usr/bin:/bin:${PATH:-}"
REPO="${PII_REPO:-/projects/pi-improved}"   # PII_REPO exists so the alarm can be TESTED on a fabricated queue
cd "$REPO" || { echo "QUEUE ALARM: cannot cd into $REPO"; exit 0; }
IDLE_MIN=${PII_IDLE_MIN:-12}

control=$(head -1 _tmp/autonomy/control 2>/dev/null || echo RUNNING)
case "$control" in
  RUNNING*) ;;
  *) exit 0 ;;                     # an explicit stop is not a fault
esac

# 1. is there a dispatchable row?
pick=$(python3 scripts/autonomy/state.py pick 2>&1 | head -1)
# pick prints a human line ("pick: skip parked A78 ... -> dispatching A99" or "pick: NONE"), so extract the id rather than
# matching the whole line: the first version matched ^A[0-9] and therefore NEVER fired (found by its own positive control).
row=$(printf '%s' "$pick" | grep -oE 'dispatching A[0-9]+' | grep -oE 'A[0-9]+' | head -1)
[ -n "$row" ] || exit 0          # nothing pending, or all parked — both are named states

# 2. is anything alive?
alive=$(python3 - <<'PY'
import glob
n=[]
for d in glob.glob('/proc/[0-9]*'):
    try: a=[x.decode('utf8','replace') for x in open(d+'/cmdline','rb').read().split(b'\0') if x]
    except Exception: continue
    if not a: continue
    if a[0].endswith('/claude') or a[0].endswith('/pi'): n.append('worker')
    elif len(a)>=2 and a[0].endswith('bash') and a[1].split('/')[-1] in ('cycle.sh','drain.sh'):
        n.append(a[1].split('/')[-1])
print(','.join(sorted(set(n))))
PY
)
# PII_ALARM_IGNORE_ALIVE=1 exists ONLY so a fabricated queue can be tested while real work runs elsewhere; the cron
# never sets it, so production always honours the liveness check.
if [ -n "$alive" ] && [ "${PII_ALARM_IGNORE_ALIVE:-0}" != "1" ]; then exit 0; fi   # work in flight

# 3. how long has the tree been untouched by a cycle?
newest=$(ls -t _tmp/autonomy/logs/cycle-*.log 2>/dev/null | head -1)
if [ -n "$newest" ]; then
  age_min=$(( ( $(date +%s) - $(stat -c %Y "$newest") ) / 60 ))
else
  age_min=$IDLE_MIN
fi
[ "$age_min" -lt "$IDLE_MIN" ] && exit 0   # a cycle just ran; give the next firing its window

printf 'QUEUE IDLE: %s is dispatchable, no worker and no cycle alive, last cycle log %s min ago (control: %s). The dispatcher is not doing its job.\n' \
  "$row" "$age_min" "$(printf '%s' "$control" | cut -c1-40)"
