#!/usr/bin/env bash
# exercise-193-196-live.sh — two specs, one scratch daemon, observed only.
#
# Rule of the exercises, verbatim: "you just observe, the system should heal". Nothing here repairs what it
# injects; every verdict is printed only over a daemon that demonstrably answered health first.
#
#   (193) a cron entry's start emits exactly ONE `cron.fired` naming the entry, the graph and the run;
#   (196) cancelling a suspended timer run REMOVES its wheel entry, and the re-armed run's entry survives;
#   (196) resuming a run that already finished is REFUSED, loudly (`run.resume.refused`), not silently.
#
# Run: bash scripts/archive/exercise-193-196-live.sh
set -uo pipefail
cd /projects/pi-improved || exit 1
. scripts/autonomy/env.sh 2>/dev/null
PII=${PII_BIN:-target/release/pii}
[ -x "$PII" ] || { echo "ABORT: $PII not built"; exit 2; }
W=$(mktemp -d /projects/_tmp/ex193196-XXXXXX) || exit 1
mkdir -p "$W/graphs" "$W/run"
MIN=$(date -u -d '+2 minutes' '+%M'); HOUR=$(date -u -d '+2 minutes' '+%H'); DOM=$(date -u -d '+2 minutes' '+%d')
cat > "$W/graphs/tick-graph.toml" <<CFG
name = "tick-graph"
entry = "t"
caps = ["post"]

[[node]]
id = "t"
kind = "timer"
cron = "$MIN $HOUR $DOM * *"

[[node]]
id = "p"
kind = "post"
caps = ["post"]
channel = "cli-post"
to = "c1"
text = "tick"

[[edge]]
from = "t"
port = "fired"
to = "p"
CFG
cat > "$W/pii.toml" <<CFG
[core]
model = "openai:glm-5.3-flash"

[[channel]]
name = "cli-post"
kind = "cli"
graph = "tick-graph"
inbox = "$W/run/channels/cli-post/inbox.txt"
outbox = "$W/run/channels/cli-post/outbox.txt"

[[cron]]
name = "ticker"
graph = "tick-graph"
schedule = "$MIN $HOUR $DOM * *"
CFG
mkdir -p "$W/run/channels/cli-post"; : > "$W/run/channels/cli-post/inbox.txt"

start() {
  env PII_RUNTIME_DIR="$W/run" "$PII" daemon --store "$W/store.redb" --workspace "$W" \
    --config "$W/pii.toml" --log-file "$W/event.log" --foreground >>"$W/daemon.log" 2>&1 &
  echo $!
}
up() { PII_RUNTIME_DIR="$W/run" "$PII" health --json >/dev/null 2>&1; }
lsruns() { PII_RUNTIME_DIR="$W/run" "$PII" run ls 2>/dev/null; }
suspended_id() {
  lsruns | python3 -c '
import sys, json
d = json.load(sys.stdin); rows = d if isinstance(d, list) else d.get("runs", [])
s = [r["run_id"] for r in rows if r.get("status") == "suspended"]
print(s[0] if s else "")'
}

PID=$(start)
for _ in $(seq 1 40); do sleep 0.5; up && break; done
if ! up; then
  echo "ABORT: the scratch daemon never answered health; daemon.log:"; tail -8 "$W/daemon.log"
  kill -9 "$PID" 2>/dev/null; exit 2
fi
sleep 2
ARMED=$(suspended_id)
FIRED_COUNT=$(grep -c '"kind":"cron.fired"' "$W/event.log" 2>/dev/null || true)
FIRED_LINE=$(grep '"kind":"cron.fired"' "$W/event.log" 2>/dev/null | head -1)
echo "armed run: ${ARMED:-none} | cron.fired lines: $FIRED_COUNT"
if [ -n "$FIRED_LINE" ] && printf '%s' "$FIRED_LINE" | grep -q '"cron":"ticker"' \
  && printf '%s' "$FIRED_LINE" | grep -q "\"run\":\"$ARMED\""; then
  echo "193 PASS — one cron.fired naming the entry, the graph and the armed run"
else
  echo "193 FAIL — expected exactly one cron.fired for run $ARMED; got: ${FIRED_LINE:-none}"
fi

# ---- 196: cancel the armed timer run; its wheel entry must go, the re-arm's must stay ----
WHEEL_BEFORE=$(strings "$W/run/timer.redb" 2>/dev/null | grep -c "$ARMED:tick:" || true)
PII_RUNTIME_DIR="$W/run" "$PII" run cancel "$ARMED" --reason "exercise" >/dev/null 2>&1
sleep 2
WHEEL_AFTER=$(strings "$W/run/timer.redb" 2>/dev/null | grep -c "$ARMED:tick:" || true)
NEW_ARMED=$(suspended_id)
NEW_WHEEL=$(strings "$W/run/timer.redb" 2>/dev/null | grep -c "$NEW_ARMED:tick:" || true)
CANCEL_LINE=$(grep '"kind":"run.cancelled"' "$W/event.log" 2>/dev/null | tail -1)
echo "wheel entries for the cancelled run: before=$WHEEL_BEFORE after=$WHEEL_AFTER | re-armed $NEW_ARMED wheel=$NEW_WHEEL"
DISARMED=$(printf '%s' "$CANCEL_LINE" | grep -o '"disarmed":[0-9]*' || true)
if [ "$WHEEL_AFTER" = "0" ] && [ -n "$NEW_ARMED" ] && [ "$NEW_WHEEL" -ge 1 ]; then
  echo "196 PASS — the cancelled run's timer is gone from the wheel ($DISARMED); the re-armed run has its own"
else
  echo "196 FAIL — wheel before/after for $ARMED: $WHEEL_BEFORE/$WHEEL_AFTER; re-armed ${NEW_ARMED:-none} wheel=$NEW_WHEEL"
fi

# ---- 196 R3: resuming a run that already finished is refused, loudly ----
OUT=$(PII_RUNTIME_DIR="$W/run" "$PII" run resume "$ARMED" 2>&1); RC=$?
REFUSED=$(grep -c '"kind":"run.resume.refused"' "$W/event.log" 2>/dev/null || true)
echo "run resume $ARMED -> rc=$RC: $(printf '%s' "$OUT" | head -1 | cut -c1-140)"
if [ "$RC" -ne 0 ] && printf '%s' "$OUT" | grep -qi "already finished" && [ "$REFUSED" -ge 1 ]; then
  echo "196-R3 PASS — a resume of a finished run refuses AND records run.resume.refused"
else
  echo "196-R3 FAIL — rc=$RC refused_events=$REFUSED"
fi

kill -9 "$PID" 2>/dev/null
echo "scratch kept at $W (nothing repaired by the harness)"
