#!/usr/bin/env bash
# watchdog.sh — the loop must NEVER idle: an independent scheduler, on a 5-minute beat.
#
# The user's requirement, verbatim: "nao pode parar!!! se para deixa de ter efeito!". This container has no
# `crontab` binary and no systemd (measured 2026-10-11), so the only two schedulers available are the Hermes
# cron ticker (30 min) and this — a detached loop that beats every 5 minutes. Both go through tick.sh, which
# takes the lock before doing anything, so they cannot collide and neither is on the critical path alone.
#
# It is silent by design: tick.sh prints only on a milestone, on an open question, on a closed window or on a
# stuck cycle. Nothing here spends anything: the probe gates every dispatch.
set -uo pipefail
. "$(dirname "$(readlink -f "$0")")/env.sh"
cd /projects/pi-improved || exit 1
PIDFILE=_tmp/autonomy/watchdog.pid
if [ -f "$PIDFILE" ] && kill -0 "$(cat "$PIDFILE" 2>/dev/null)" 2>/dev/null; then
  echo "pii autonomy: watchdog already running (pid $(cat "$PIDFILE"))"; exit 0
fi
echo $$ >"$PIDFILE"
trap 'rm -f "$PIDFILE"; exit 0' TERM INT
echo "pii autonomy: watchdog started (pid $$, beat ${WATCHDOG_INTERVAL:-300}s)"
while :; do
  bash scripts/autonomy/tick.sh >/dev/null 2>&1 || true
  sleep "${WATCHDOG_INTERVAL:-300}"
done
