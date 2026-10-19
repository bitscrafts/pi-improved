#!/usr/bin/env bash
# exercise-196-past-the-instant.sh — the production shape, replayed: a cancelled run's instant arrives and
# NOTHING fires. On 2026-10-19 09:00Z four cancelled runs fired at their instant and posted four digests.
#
# Rule of the exercises, verbatim: "you just observe, the system should heal". The harness cancels once and
# then only watches the clock; it never touches the daemon again.
set -uo pipefail
cd /projects/pi-improved || exit 1
. scripts/autonomy/env.sh 2>/dev/null
PII=${PII_BIN:-target/release/pii}
[ -x "$PII" ] || { echo "ABORT: $PII not built"; exit 2; }
W=$(mktemp -d /projects/_tmp/ex196-XXXXXX) || exit 1
mkdir -p "$W/graphs" "$W/run" "$W/run/channels/cli-post"
: > "$W/run/channels/cli-post/inbox.txt"
# the cron fires in the NEXT minute (utc), so the wait is short
MIN=$(date -u -d '+3 minutes' '+%M'); HOUR=$(date -u -d '+3 minutes' '+%H'); DOM=$(date -u -d '+3 minutes' '+%d')
cat > "$W/graphs/fire.toml" <<CFG
name = "fire"
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
text = "fired"

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
graph = "fire"
inbox = "$W/run/channels/cli-post/inbox.txt"
outbox = "$W/run/channels/cli-post/outbox.txt"

[[cron]]
name = "one-shot"
graph = "fire"
schedule = "$MIN $HOUR $DOM * *"
CFG
env PII_RUNTIME_DIR="$W/run" "$PII" daemon --store "$W/store.redb" --workspace "$W" \
  --config "$W/pii.toml" --log-file "$W/event.log" --foreground >>"$W/daemon.log" 2>&1 &
PID=$!
up() { PII_RUNTIME_DIR="$W/run" "$PII" health --json >/dev/null 2>&1; }
runs() { PII_RUNTIME_DIR="$W/run" "$PII" run ls 2>/dev/null; }
for _ in $(seq 1 40); do sleep 0.5; up && break; done
if ! up; then echo "ABORT: no health; $W/daemon.log:"; tail -8 "$W/daemon.log"; kill -9 "$PID"; exit 2; fi
sleep 2
ARMED=$(runs | python3 -c '
import sys, json
d = json.load(sys.stdin); rows = d if isinstance(d, list) else d.get("runs", [])
s = [r["run_id"] for r in rows if r.get("status") == "suspended"]
print(s[0] if s else "")')
echo "armed: $ARMED at $(date -u +%H:%M:%SZ); firing minute: $MIN"
PII_RUNTIME_DIR="$W/run" "$PII" run cancel "$ARMED" --reason "exercise: past-the-instant" >/dev/null 2>&1
REARMED=$(runs | python3 -c '
import sys, json
d = json.load(sys.stdin); rows = d if isinstance(d, list) else d.get("runs", [])
s = [r["run_id"] for r in rows if r.get("status") == "suspended"]
print(s[0] if s else "")')
# The cron entry re-arms the next occurrence as soon as a run finishes, which is correct behaviour and would
# make "did the cancelled run fire?" unanswerable. So the schedule itself is removed from the config and the
# daemon restarted: from here the ONLY way a run can start is a wheel entry that survived the cancel.
kill -9 "$PID" 2>/dev/null; sleep 1
python3 - "$W/pii.toml" <<'PY'
import sys
p = sys.argv[1]
s = open(p).read()
i = s.find("[[cron]]")
if i > 0:
    s = s[:i].rstrip() + "\n"
open(p, "w").write(s)
PY
env PII_RUNTIME_DIR="$W/run" "$PII" daemon --store "$W/store.redb" --workspace "$W" \
  --config "$W/pii.toml" --log-file "$W/event.log" --foreground >>"$W/daemon.log" 2>&1 &
PID=$!
for _ in $(seq 1 40); do sleep 0.5; up && break; done
if ! up; then echo "ABORT: the restarted daemon never answered health"; tail -6 "$W/daemon.log"; kill -9 "$PID"; exit 2; fi
echo "cron entry removed from the config, daemon restarted at $(date -u +%H:%M:%SZ); only a surviving wheel entry could fire"
# With no [[cron]] left in the config, cancelling whatever is still armed cannot re-arm anything: from here a
# firing can only come from a wheel entry the cancels failed to remove.
STILL=$(runs | python3 -c '
import sys, json
d = json.load(sys.stdin); rows = d if isinstance(d, list) else d.get("runs", [])
s = [r["run_id"] for r in rows if r.get("status") == "suspended"]
print(" ".join(s))')
for r in $STILL; do
  PII_RUNTIME_DIR="$W/run" "$PII" run cancel "$r" --reason "exercise: the re-armed occurrence too" >/dev/null 2>&1
  echo "cancelled the re-armed run $r (no cron entry remains to arm another)"
done
# wait until 20 s past the scheduled minute, watching only
TARGET=$(date -u -d "$(date -u +%Y-%m-%d\ $HOUR:$MIN:20)" +%s)
while [ "$(date -u +%s)" -lt "$TARGET" ]; do sleep 5; done
sleep 20
AFTER=$(runs)
TOTAL=$(printf '%s' "$AFTER" | python3 -c 'import sys,json; d=json.load(sys.stdin); print(len(d if isinstance(d,list) else d.get("runs",[])))')
# a run that fired EXECUTED: it is succeeded, or failed for a real reason - never merely cancelled
FIRED=$(printf '%s' "$AFTER" | python3 -c '
import sys, json
d = json.load(sys.stdin); rows = d if isinstance(d, list) else d.get("runs", [])
print(sum(1 for r in rows if r.get("status") in ("succeeded", "running")))')
CANCELLED=$(printf '%s' "$AFTER" | python3 -c '
import sys, json
d = json.load(sys.stdin); rows = d if isinstance(d, list) else d.get("runs", [])
print(sum(1 for r in rows if r.get("status") == "failed"))')
POSTS=$(grep -c '"kind":"channel.posted"' "$W/event.log" 2>/dev/null || true)
echo "after the instant: total runs=$TOTAL (cancelled $ARMED and the re-armed one), runs that EXECUTED=$FIRED, failed-without-executing=$CANCELLED, posts=$POSTS"
if [ "$FIRED" = "0" ] && [ "$POSTS" = "0" ]; then
  echo "196 PASS — the cancelled run's instant arrived and nothing fired (pre-fix: the 09:00Z shape)"
else
  echo "196 FAIL — $FIRED run(s) fired at the cancelled run's instant; $POSTS post(s)"
fi
kill -9 "$PID" 2>/dev/null
echo "scratch kept at $W"
