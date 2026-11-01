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
PIDFILE="${AUTONOMY_WATCHDOG_PID:-_tmp/autonomy/watchdog.pid}"   # overridable: a test must not be read as a second instance
if [ -f "$PIDFILE" ] && kill -0 "$(cat "$PIDFILE" 2>/dev/null)" 2>/dev/null; then
  echo "pii autonomy: watchdog already running (pid $(cat "$PIDFILE"))"; exit 0
fi
echo $$ >"$PIDFILE"
trap 'rm -f "$PIDFILE"; exit 0' TERM INT
echo "pii autonomy: watchdog started (pid $$, beat ${WATCHDOG_INTERVAL:-300}s)"
# Opus review 2026-11-01: this used to be `tick.sh >/dev/null 2>&1`, and the tick's incident scan runs with --new,
# which MARKS an incident reported. So every beat for eleven days marked incidents and threw the line away: the
# record existed and nothing could read it. The line lands in a dated log now -- whether it is delivered anywhere
# is a product decision, but discarding the evidence never was one.
LOGD="${AUTONOMY_LOGD:-_tmp/autonomy/logs}"          # overridable so a test can watch the write without
TICK="${AUTONOMY_TICK:-scripts/autonomy/tick.sh}"   # touching the real logs or running the real tick
mkdir -p "$LOGD"
while :; do
  # a held tree is not the beat's to touch -- the tick can dispatch a cycle, so it waits for the holder to finish
  if bash scripts/autonomy/hold.sh check >/dev/null 2>&1; then
    bash "$TICK" >>"$LOGD/tick-$(date -u +%Y%m%d).log" 2>&1 || true
    # the incident surface that exists today is a file: the tick formats the line, this publishes it where a person
    # can read it (the delivering cron job has been disabled since 2026-10-19; Discord delivery is a later decision)
    bash scripts/autonomy/incidents-md.sh >>"$LOGD/tick-$(date -u +%Y%m%d).log" 2>&1 || true
  else
    echo "watchdog: $(bash scripts/autonomy/hold.sh check | sed 's/^hold: //') -- beat skipped" \
      >>"$LOGD/tick-$(date -u +%Y%m%d).log"
  fi
  sleep "${WATCHDOG_INTERVAL:-300}s"
done
