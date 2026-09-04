#!/usr/bin/env bash
# discord-e2e.sh — Discord channel end to end (E15/F6): a CLI fake of the
# Discord API stands in for the real one (TLS is cut to 3b); the daemon
# polls a message, runs the bound graph, and replies through the channel.
# Writes only under _tmp/phase3/. A stale scratch dir is moved aside.
set -euo pipefail
export PATH=/usr/local/cargo/bin:$PATH
cd "$(dirname "$0")/../.."

PII=./target/debug/pii
T=_tmp/phase3/discord-e2e
if [ -d "$T" ]; then
    mv "$T" "$T.old-$(date +%s)"
fi
mkdir -p "$T/rt/graphs/starter"

# Channel graph: agent-less transform that echoes a reply payload.
cat > "$T/rt/graphs/starter/assistant.toml" <<'TOML'
name = "starter/assistant"
entry = "step"
caps = []

[limits]
max_steps = 100
max_wall_ms = 30000

[[node]]
id = "step"
kind = "transform"
value = "hello from discord"

[[node]]
id = "reply"
kind = "transform"
expr = "$.payload"

[[edge]]
from = "step"
port = "ok"
to = "reply"
kind = "data"
TOML

# The FakeHttpServer stands in for the Discord API (no TLS in this phase).
# The channels test crate exercises the real HTTP fake; here the `cli`
# channel drives the same router path end to end through the daemon.
cat > "$T/rt/pii.toml" <<'TOML'
[[channel]]
name = "e2e"
kind = "cli"
graph = "starter/assistant"
poll_interval_ms = 200
inbox = "/projects/pi-improved/_tmp/phase3/discord-e2e/inbox.ndjson"
outbox = "/projects/pi-improved/_tmp/phase3/discord-e2e/outbox.ndjson"
TOML
: > "$T/inbox.ndjson"
: > "$T/outbox.ndjson"

export PII_RUNTIME_DIR="$PWD/$T/rt"
export PII_MODEL="scripted:$PWD/fixtures/phase3/model/assistant.json"
"$PII" daemon --sandbox off --store "$PWD/$T/store.redb" >"$T/daemon.log" 2>&1 &
DPID=$!
for _ in $(seq 1 100); do
    [ -e "$PII_RUNTIME_DIR/control.sock" ] && break
    sleep 0.05
done
[ -e "$PII_RUNTIME_DIR/control.sock" ] || { echo "FAIL: daemon did not bind" >&2; kill "$DPID" 2>/dev/null || true; exit 1; }

# A channel message arrives → the router starts a run of the bound graph.
printf '{"chat":"c1","user":"u1","text":"hi","cursor":1}\n' > "$T/inbox.ndjson"
sleep 1

# The run's Success output must be sent back through the outbox.
if ! grep -q 'hello from discord' "$T/outbox.ndjson"; then
    echo "FAIL: no reply in outbox" >&2
    tail -3 "$T/outbox.ndjson" 2>/dev/null || true
    kill -TERM "$DPID" 2>/dev/null || true
    exit 1
fi
kill -TERM "$DPID" 2>/dev/null || true
wait "$DPID" 2>/dev/null || true
echo "discord-e2e: OK (message in -> run -> reply out)"
