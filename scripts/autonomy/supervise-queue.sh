#!/usr/bin/env bash
# supervise-queue.sh — a supervisor that does not depend on cron, a wrapper, or any path but its own.
#
# WHY THIS EXISTS (user, twice tonight, verbatim): "again! queue stopped without specific order! again a queue must drain!"
# Every stop tonight came from the LAYER AROUND the queue, never from the queue:
#   · a cron whose script name resolved to a file that did not exist
#   · a drain whose `if` was unterminated, so every firing died on a syntax error
#   · a */30 cadence that bailed on a lock instead of waiting
# A supervisor that loops in-process removes that whole class: it re-checks every 60 s and dispatches when the queue has a
# dispatchable head and nothing is working it. It is started detached (setsid nohup) so a session end cannot orphan the queue.
#
# It never starts a second worker: the drain's flock is the mutual exclusion, and this loop only calls drain when no
# claude/pi worker and no cycle are alive.
#
# Stop it explicitly:  echo PAUSED > _tmp/autonomy/control   (the loop exits, and the queue is then a named stop)
set -uo pipefail
export PATH="/root/.local/bin:/usr/local/cargo/bin:/usr/local/bin:/usr/bin:/bin:${PATH:-}"
export PII_WORKER="${PII_WORKER:-claude}"
export CLAUDE_MODEL="${CLAUDE_MODEL:-claude-haiku-5-5}"
REPO="${PII_REPO:-/projects/pi-improved}"
cd "$REPO" || exit 1
LOG=_tmp/autonomy/logs/supervisor.log
mkdir -p _tmp/autonomy/logs
INTERVAL=${PII_SUPERVISE_INTERVAL:-60}

log() { printf '%s %s\n' "$(date -u +%FT%TZ)" "$*" >> "$LOG"; }

log "supervisor start pid=$$ worker=$PII_WORKER model=$CLAUDE_MODEL interval=${INTERVAL}s"
while :; do
  ctl=$(head -1 _tmp/autonomy/control 2>/dev/null || echo RUNNING)
  case "$ctl" in
    RUNNING*) ;;
    *) log "control is not RUNNING — supervisor exiting: $ctl"; exit 0 ;;
  esac

  pick=$(python3 scripts/autonomy/state.py pick 2>&1 | head -1)
  row=$(printf '%s' "$pick" | grep -oE '(^|dispatching )A[0-9]+' | grep -oE 'A[0-9]+' | head -1)
  if [ -z "$row" ]; then
    sleep "$INTERVAL"; continue
  fi

  alive=$(python3 - <<'PY'
import glob
n=[]
for d in glob.glob('/proc/[0-9]*'):
    try: a=[x.decode('utf8','replace') for x in open(d+'/cmdline','rb').read().split(b'\0') if x]
    except Exception: continue
    if not a: continue
    if a[0].endswith('/claude') or a[0].endswith('/pi'): n.append('worker')
    elif len(a)>=2 and a[0].endswith('bash') and a[1].split('/')[-1] in ('cycle.sh','drain.sh'): n.append(a[1].split('/')[-1])
print(','.join(sorted(set(n))))
PY
)
  if [ -n "$alive" ]; then
    sleep "$INTERVAL"; continue
  fi

  log "dispatch: $row is pending and nothing is alive — starting a drain"
  out=$(MAX_CYCLES="${MAX_CYCLES:-4}" bash scripts/autonomy/drain.sh 2>&1 | tail -6)
  [ -n "$out" ] && printf '%s\n' "$out" | while IFS= read -r l; do log "  $l"; done
  sleep 5
done
