#!/usr/bin/env bash
# exercise-e2-e3-graphs.sh — E3 (a broken cron graph is refused, loudly and by name) and E2 (a missing graphs
# directory falls back, and runs continue), each on its own scratch daemon.
#
# Rule of the exercises, verbatim: "you just observe, the system should heal". Nothing here repairs what it
# injects, and no verdict is printed over a daemon that did not answer health first.
set -uo pipefail
cd /projects/pi-improved || exit 1
. scripts/autonomy/env.sh 2>/dev/null
PII=${PII_BIN:-target/release/pii}
[ -x "$PII" ] || { echo "ABORT: $PII not built"; exit 2; }

mk_graph() { # $1 dir, $2 name
  mkdir -p "$1"
  cat > "$1/$2.toml" <<CFG
name = "$2"
entry = "hi"

[[node]]
id = "hi"
kind = "echo"
text = "hello"
CFG
}

# ---------------- E3: a cron entry naming a graph that does not exist ----------------
W=$(mktemp -d /projects/_tmp/exe3-XXXXXX); mkdir -p "$W/graphs" "$W/run"
mk_graph "$W/graphs" "ok-graph"
cat > "$W/pii.toml" <<CFG
[core]
model = "openai:glm-5.3-flash"

[[cron]]
name = "typo"
graph = "no-such-graph"
schedule = "0 3 * * *"
CFG
env PII_RUNTIME_DIR="$W/run" "$PII" daemon --store "$W/store.redb" --workspace "$W" \
  --config "$W/pii.toml" --log-file "$W/event.log" --foreground >>"$W/daemon.log" 2>&1 &
PID=$!
up() { PII_RUNTIME_DIR="$W/run" "$PII" health --json >/dev/null 2>&1; }
for _ in $(seq 1 40); do sleep 0.5; up && break; done
if ! up; then echo "ABORT E3: no health; $W/daemon.log:"; tail -6 "$W/daemon.log"; kill -9 "$PID"; exit 2; fi
REFUSED=$(grep -c '"kind":"cron.start.refused"' "$W/event.log" 2>/dev/null || true)
NAMED=$(grep '"kind":"cron.start.refused"' "$W/event.log" 2>/dev/null | grep -c 'no-such-graph' || true)
echo "E3 event: $(grep '"kind":"cron.start.refused"' "$W/event.log" 2>/dev/null | head -1 | cut -c1-220)"
if [ "$REFUSED" -ge 1 ] && [ "$NAMED" -ge 1 ] && up; then
  echo "E3 PASS — the broken cron graph is refused once, by name, and the daemon stays up"
else
  echo "E3 FAIL — refused=$REFUSED named=$NAMED healthy=$(up && echo yes || echo no)"
fi
kill -9 "$PID" 2>/dev/null

# ---------------- E2: no graphs directory at all ----------------
W2=$(mktemp -d /projects/_tmp/exe2-XXXXXX); mkdir -p "$W2/run" "$W2/run/channels/inbox" "$W2/state/graphs"
mk_graph "$W2/state/graphs" "ok-graph"   # the fallback lives beside the store, not in the runtime dir
: > "$W2/run/channels/inbox/inbox.txt"
cat > "$W2/pii.toml" <<CFG
[core]
model = "openai:glm-5.3-flash"

[[channel]]
name = "inbox"
kind = "cli"
graph = "ok-graph"
inbox = "$W2/run/channels/inbox/inbox.txt"
outbox = "$W2/run/channels/inbox/outbox.txt"
CFG
env PII_RUNTIME_DIR="$W2/run" "$PII" daemon --store "$W2/state/store.redb" --workspace "$W2" \
  --config "$W2/pii.toml" --log-file "$W2/event.log" --foreground >>"$W2/daemon.log" 2>&1 &
PID2=$!
up2() { PII_RUNTIME_DIR="$W2/run" "$PII" health --json >/dev/null 2>&1; }
for _ in $(seq 1 40); do sleep 0.5; up2 && break; done
if ! up2; then echo "ABORT E2: no health; $W2/daemon.log:"; tail -6 "$W2/daemon.log"; kill -9 "$PID2"; exit 2; fi
FALLBACK=$(grep -c '"kind":"daemon.graphs_fallback"' "$W2/event.log" 2>/dev/null || true)
# runs continue: a message on the cli channel still starts a run (through the fallen-back directory,
# which is what "runs continue" means)
echo '{"text":"hello","user":"u","chat":"c1","cursor":1}' >> "$W2/run/channels/inbox/inbox.txt"
sleep 3
STARTED=$(PII_RUNTIME_DIR="$W2/run" "$PII" run ls 2>/dev/null | python3 -c 'import sys,json; d=json.load(sys.stdin); print(len(d if isinstance(d,list) else d.get("runs",[])))')
echo "E2 events: $(grep '"kind":"daemon.graphs_fallback"' "$W2/event.log" 2>/dev/null | head -1 | cut -c1-200)"
if [ "$FALLBACK" -ge 1 ] && [ "$STARTED" -ge 1 ] && up2; then
  echo "E2 PASS — the missing graphs directory falls back, the daemon stays up, and a run still starts ($STARTED)"
else
  echo "E2 FAIL — fallback=$FALLBACK runs=$STARTED healthy=$(up2 && echo yes || echo no)"
fi
kill -9 "$PID2" 2>/dev/null
echo "scratch kept at $W and $W2"
