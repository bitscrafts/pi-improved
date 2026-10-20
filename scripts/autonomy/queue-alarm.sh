#!/usr/bin/env bash
# queue-alarm.sh — an alarm when the queue has items and is stopped. ONCE per fault, then silent until it changes.
#
# User, verbatim (2026-10-19/20): "if i dont ask we would be hours without knowing nothing about the queue!" and
# "alarm when with items in the queue and its stopped (because there is no alarm when queue is empty, it must inform
# once and bypass all further cron calls)".
#
# Behaviour, in three sentences:
#   1. A dispatchable head with no worker alive is a FAULT → print ONE line.
#   2. The same fault does not print again: its signature is remembered and every later firing is silent (bypass),
#      so a five-minute cron cannot turn one stopped queue into twelve messages an hour.
#   3. When the fault CLEARS (a worker appears, the queue empties, or the operator pauses it), the signature is erased,
#      so the next real stoppage alarms immediately.
#
# Silence is therefore meaningful: no output = either healthy, or the same fault has already been reported.
#
# Env: PII_IDLE_MIN (default 12) — minutes without a cycle log before an idle head counts as stopped.
#      PII_REPO       — run against another tree (testing only).
#      PII_ALARM_IGNORE_ALIVE=1, PII_ALARM_RESET=1 — test hooks; the cron sets neither.
set -uo pipefail
export PATH="/root/.local/bin:/usr/local/cargo/bin:/usr/local/bin:/usr/bin:/bin:${PATH:-}"
REPO="${PII_REPO:-/projects/pi-improved}"
cd "$REPO" || { echo "QUEUE ALARM: cannot cd into $REPO"; exit 0; }
IDLE_MIN=${PII_IDLE_MIN:-12}
STATE=_tmp/autonomy/alarm.state
mkdir -p _tmp/autonomy 2>/dev/null

# The alarm is a latch: this function is the ONLY place that writes it.
remember() { printf '%s\n' "$1" > "$STATE" 2>/dev/null; }
forget()   { : > "$STATE" 2>/dev/null; }
last()     { head -1 "$STATE" 2>/dev/null | cut -c1-200; }

[ "${PII_ALARM_RESET:-0}" = "1" ] && forget

# ---- condition 1: an explicit stop is not a fault -----------------------------------------------
control=$(head -1 _tmp/autonomy/control 2>/dev/null || echo RUNNING)
case "$control" in
  RUNNING*) ;;
  *) forget; exit 0 ;;
esac

# ---- condition 2: is there a dispatchable row? ---------------------------------------------------
# pick emits EITHER a tab-separated row (starts with the id) OR "… -> dispatching A99". Both shapes were read off the
# command after two earlier versions matched one each and never fired.
pick=$(python3 scripts/autonomy/state.py pick 2>&1 | head -1)
row=$(printf '%s' "$pick" | grep -oE '(^|dispatching )A[0-9]+' | grep -oE 'A[0-9]+' | head -1)
if [ -z "$row" ]; then
  forget                     # nothing pending, or every open row parked: a named state, and it clears the latch
  exit 0
fi

# ---- condition 3: is anything alive? -------------------------------------------------------------
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
if [ -n "$alive" ] && [ "${PII_ALARM_IGNORE_ALIVE:-0}" != "1" ]; then
  forget                     # work in flight: the fault has cleared, so a later stoppage may alarm again
  exit 0
fi

# ---- condition 4: how long since a cycle ran? ----------------------------------------------------
newest=$(ls -t _tmp/autonomy/logs/cycle-*.log 2>/dev/null | head -1)
if [ -n "$newest" ]; then
  age_min=$(( ( $(date +%s) - $(stat -c %Y "$newest") ) / 60 ))
else
  age_min=$IDLE_MIN
fi
if [ "$age_min" -lt "$IDLE_MIN" ]; then
  forget                     # the last cycle is recent; give the next firing its window
  exit 0
fi

# ---- the fault -----------------------------------------------------------------------------------
sig="idle:$row"
if [ "$(last)" = "$sig" ]; then
  exit 0                     # already reported this exact fault: BYPASS every further firing until it changes
fi
remember "$sig"
printf 'QUEUE STOPPED: %s is dispatchable, no worker and no cycle alive, last cycle log %s min ago. Reported once; further firings stay silent until this changes.\n' \
  "$row" "$age_min"
