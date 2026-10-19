#!/usr/bin/env bash
# exercise-e5-poll-retry-after.sh — E5: a 429 must pace the inbound poll by what the provider said.
#
# Rule of the exercises, verbatim from the user: "you just observe, the system should heal". This harness
# starts a fake Discord that answers every poll with 429 + `retry_after: 2`, starts a SCRATCH daemon
# (never the live one) on a scratch store with poll_interval_ms = 100, and reads the daemon's own event
# log. It repairs nothing.
#
# The number that decides it: the GAP between consecutive `channel.poll.failed` lines.
#   before spec 194: ~100 ms  (the configured interval; measured on production 2026-10-19: ~1.2 s with the
#                             shipped 1200 ms interval, 22 polls in 3 minutes)
#   after spec 194:  >= 1900 ms (Discord asked for 2000)
# A verdict is printed only over a daemon that demonstrably answered health first.
set -uo pipefail
cd /projects/pi-improved || exit 1
. scripts/autonomy/env.sh 2>/dev/null
PII=${PII_BIN:-target/release/pii}
[ -x "$PII" ] || { echo "ABORT: $PII not built (cargo build --release -p pii-cli)"; exit 2; }
W=$(mktemp -d /projects/_tmp/ex194-XXXXXX) || exit 1
mkdir -p "$W/graphs" "$W/run"
PORT=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')

cat > "$W/fake_discord.py" <<'PY'
import http.server, json, sys, threading
PORT = int(sys.argv[1])
class H(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a):  # keep the harness output clean; the daemon's log is the evidence
        pass
    def do_GET(self):
        # every poll is rate limited; the body is Discord's own shape
        body = json.dumps({"message": "You are being rate limited.", "retry_after": 2, "global": False}).encode()
        self.send_response(429)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
http.server.HTTPServer(("127.0.0.1", PORT), H).serve_forever()
PY
python3 "$W/fake_discord.py" "$PORT" &
FAKE=$!
sleep 0.7

cat > "$W/pii.toml" <<CFG
[core]
model = "openai:glm-5.3-flash"

[[channel]]
name = "discord"
kind = "discord"
graph = "discord-digest"
poll_interval_ms = 100
hosts = ["127.0.0.1:$PORT"]

[net]
allow_private = true
grants = ["127.0.0.1:$PORT"]
CFG

start() {
  env PII_RUNTIME_DIR="$W/run" PII_DISCORD_TOKEN=tok PII_DISCORD_CHANNEL_ID=c1 \
    PII_DISCORD_API_BASE="http://127.0.0.1:$PORT" \
    "$PII" daemon --store "$W/store.redb" --workspace "$W" --config "$W/pii.toml" \
    --log-file "$W/event.log" --foreground >>"$W/daemon.log" 2>&1 &
  echo $!
}
up() { PII_RUNTIME_DIR="$W/run" "$PII" health --json >/dev/null 2>&1; }

PID=$(start)
for _ in $(seq 1 40); do sleep 0.5; up && break; done
if ! up; then
  echo "ABORT: the scratch daemon never answered health; daemon.log:"; tail -8 "$W/daemon.log"
  kill -9 "$PID" "$FAKE" 2>/dev/null; exit 2
fi
echo "daemon up (pid $PID) against a fake Discord on 127.0.0.1:$PORT answering 429 retry_after=2"
sleep 7   # ~3 paced polls, or ~70 unpaced ones

kill -9 "$PID" 2>/dev/null; kill -9 "$FAKE" 2>/dev/null

python3 - "$W/event.log" <<'PY'
import json, sys
ts, texts = [], []
for line in open(sys.argv[1], errors="replace"):
    try: d = json.loads(line)
    except Exception: continue
    if d.get("kind") == "channel.poll.failed":
        ts.append(d.get("ts_ms", 0))
        texts.append(((d.get("fields") or {}).get("error") or ""))
ts.sort()
gaps = [b - a for a, b in zip(ts, ts[1:])]
print(f"poll failures in 7 s: {len(ts)}")
print(f"gaps ms: {gaps}")
print("error text sample:", texts[-1][:160] if texts else "(none)")
ok = len(ts) >= 2 and min(gaps) >= 1900 if gaps else False
carries = bool(texts) and "retry_after" in texts[-1]
print()
if ok and carries:
    print("E5 PASS — every poll after the first waited for the 2 s the provider asked for, and the error carries it")
elif not carries:
    print("E5 FAIL — the poll error does not carry retry_after: this binary predates spec 194")
elif len(ts) < 2:
    print("E5 INCONCLUSIVE — fewer than two failures observed in the window")
else:
    print(f"E5 FAIL — polls every {min(gaps)} ms while the provider asked for 2000 ms")
PY
echo "scratch kept at $W (injected fault, never repaired by the harness)"
