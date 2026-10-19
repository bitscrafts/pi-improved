#!/usr/bin/env bash
# exercise-187-restarts.sh (v2) — the DECISIVE test of "one cron entry, one armed run" (spec 185/187).
#
# WHAT v1 GOT WRONG, and both lessons are kept here in the code:
#   1. it wrote a `[store]` table in the scratch config. The schema has no such field — the store is the `--store` FLAG —
#      so the daemon refused the config and exited, and every reading was "?".
#   2. it then printed "VERDICT: 187 FAILS" over a process that never started. A harness that reports red against a broken
#      setup is worse than no harness: it makes a working fix look broken and a broken one look tested.
# So: no [store] table, and the script ABORTS unless the daemon demonstrably answers health.
#
# Run:  bash scripts/exercise-187-restarts.sh
set -uo pipefail
cd /projects/pi-improved || exit 1
. scripts/autonomy/env.sh 2>/dev/null
PII=/root/.local/bin/pii
W=$(mktemp -d /projects/_tmp/exercise187-XXXXXX) || exit 1
echo "scratch workspace: $W"
mkdir -p "$W/graphs" "$W/run"
cp graphs/discord-digest.toml "$W/graphs/" 2>/dev/null || true

cat > "$W/pii.toml" <<CFG
[[channel]]
name = "discord"
kind = "discord"
graph = "discord-digest"

[[cron]]
name = "digest"
graph = "discord-digest"
schedule = "0 12 * * *"
input = { to = "scratch" }
CFG

start() {
  PII_RUNTIME_DIR="$W/run" "$PII" daemon --store "$W/store.redb" --workspace "$W" --config "$W/pii.toml" --foreground \
    >> "$W/daemon.log" 2>&1 &
  echo $!
}

up() { PII_RUNTIME_DIR="$W/run" "$PII" health --json >/dev/null 2>&1; }

armed() {
  PII_RUNTIME_DIR="$W/run" "$PII" run ls --status suspended 2>/dev/null | python3 -c '
import sys, json
d = json.load(sys.stdin)
r = d if isinstance(d, list) else d.get("runs", [])
print(len(r))
'
}

PID=$(start); sleep 8
if ! up; then
  echo "ABORT: the scratch daemon is not answering health. Its log:"
  tail -6 "$W/daemon.log"
  exit 2
fi
echo "start 1: pid=$PID  armed=$(armed)"

for i in 2 3 4; do
  kill -9 "$PID" 2>/dev/null
  sleep 2
  PID=$(start); sleep 8
  if ! up; then
    echo "ABORT at start $i: daemon not answering health. Log:"
    tail -6 "$W/daemon.log"
    exit 2
  fi
  echo "start $i: pid=$PID  armed=$(armed)"
done

FINAL=$(armed)
echo "--- final: armed=$FINAL  (before 187 it grew to 4; after 187 it must stay 1) ---"
PII_RUNTIME_DIR="$W/run" "$PII" run ls --status suspended 2>/dev/null | head -c 300
echo
if [ "$FINAL" = "1" ]; then
  echo "VERDICT: 187 HOLDS — four starts, one armed run"
else
  echo "VERDICT: 187 FAILS — four starts, $FINAL armed runs"
fi
kill -9 "$PID" 2>/dev/null
echo "scratch daemon stopped; workspace kept at $W"
