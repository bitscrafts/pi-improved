#!/usr/bin/env bash
# exercise-e6-send-pacing.sh — E6: a refused post is paced by the retry_after it carries, and a post still lands.
#
# Rule of the exercises, verbatim: "you just observe, the system should heal". This harness starts a fake
# Discord that 429s the FIRST post (retry_after 1 s) and accepts the rest, a SCRATCH daemon (never the live
# one) whose CLI channel turns three inbox lines into three runs, and reads the daemon's own events plus the
# fake's dial counter. Nothing is repaired by the harness.
#
# The numbers that decide it:
#   run 1 (t=0)    -> dial #1 -> 429, the hub records a 3 s pace, the run fails (channel.post_refused).
#   run 2 (t=+0.8) -> INSIDE the window: NO dial (counter stays 1), refused as "paced by the provider".
#   run 3 (t=+3.6) -> AFTER the window: dial #2 -> 200, the post LANDS (delivered).
# A verdict is printed only over a daemon that demonstrably answered health first.
set -uo pipefail
cd /projects/pi-improved || exit 1
. scripts/autonomy/env.sh 2>/dev/null
PII=${PII_BIN:-target/release/pii}
[ -x "$PII" ] || { echo "ABORT: $PII not built"; exit 2; }
W=$(mktemp -d /projects/_tmp/ex195-XXXXXX) || exit 1
mkdir -p "$W/graphs" "$W/run"
PORT=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')

cat > "$W/fake_discord.py" <<'PY'
import http.server, json, sys, threading
PORT, STATE = int(sys.argv[1]), sys.argv[2]
lock = threading.Lock()

def bump(field):
    with lock:
        try: st = json.load(open(STATE))
        except Exception: st = {"posts": 0, "ok": 0}
        st[field] = st.get(field, 0) + 1
        json.dump(st, open(STATE, "w"))

class H(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def note(self, m):
        import time
        open(str(STATE) + ".reqs", "a").write(f"{int(time.time()*1000)} {m} {self.path}\n")
    def do_GET(self):
        self.note("GET")
        # the poll endpoint wants an ARRAY; the shape probe wants a channel object
        body = b"[]" if self.path.split("?")[0].endswith("/messages") else json.dumps({"type": 0}).encode()
        self.send_response(200); self.send_header("Content-Length", str(len(body))); self.end_headers()
        self.wfile.write(body)
    def do_POST(self):
        self.note("POST")
        bump("posts")
        try: first = json.load(open(STATE)).get("posts", 1) == 1
        except Exception: first = True
        if first:
            body = json.dumps({"message": "You are being rate limited.", "retry_after": 3, "global": False}).encode()
            self.send_response(429)
        else:
            bump("ok"); body = b"{}"
            self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body))); self.end_headers()
        self.wfile.write(body)
http.server.HTTPServer(("127.0.0.1", PORT), H).serve_forever()
PY
echo '{"posts": 0, "ok": 0}' > "$W/state.json"
python3 "$W/fake_discord.py" "$PORT" "$W/state.json" &
FAKE=$!
sleep 0.7

cat > "$W/graphs/post-once.toml" <<CFG
name = "post-once"
entry = "p"
caps = ["post"]

[[node]]
id = "p"
kind = "post"
caps = ["post"]
channel = "discord"
to = "c1"
text = "paced test"
timeout_ms = 3000
CFG

cat > "$W/pii.toml" <<CFG
[core]
model = "openai:glm-5.3-flash"

[[channel]]
name = "inbox"
kind = "cli"
graph = "post-once"
poll_interval_ms = 100
inbox = "$W/run/channels/inbox/inbox.txt"
outbox = "$W/run/channels/inbox/outbox.txt"

[[channel]]
name = "discord"
kind = "discord"
graph = "post-once"
hosts = ["127.0.0.1:$PORT"]

[net]
allow_private = true
grants = ["127.0.0.1:$PORT"]
CFG
mkdir -p "$W/run/channels/inbox"
: > "$W/run/channels/inbox/inbox.txt"

start() {
  env PII_RUNTIME_DIR="$W/run" PII_DISCORD_TOKEN=tok PII_DISCORD_CHANNEL_ID=c1 \
    PII_DISCORD_API_BASE="http://127.0.0.1:$PORT" \
    "$PII" daemon --store "$W/store.redb" --workspace "$W" --config "$W/pii.toml" \
    --log-file "$W/event.log" --foreground >>"$W/daemon.log" 2>&1 &
  echo $!
}
up() { PII_RUNTIME_DIR="$W/run" "$PII" health --json >/dev/null 2>&1; }
dials() { python3 -c "import json;print(json.load(open('$W/state.json')).get('posts',0))"; }
delivered() { python3 -c "import json;print(json.load(open('$W/state.json')).get('ok',0))"; }

PID=$(start)
for _ in $(seq 1 40); do sleep 0.5; up && break; done
if ! up; then
  echo "ABORT: the scratch daemon never answered health; daemon.log:"; tail -8 "$W/daemon.log"
  kill -9 "$PID" "$FAKE" 2>/dev/null; exit 2
fi
echo "daemon up (pid $PID); fake Discord on 127.0.0.1:$PORT (first post 429 retry_after=1, then 200)"

printf '%s\n' '{"text":"hello one","user":"u","chat":"c1","cursor":1}' >> "$W/run/channels/inbox/inbox.txt"; sleep 0.6
D1=$(dials)
printf '%s\n' '{"text":"hello two","user":"u","chat":"c1","cursor":2}' >> "$W/run/channels/inbox/inbox.txt"; sleep 0.5
D2=$(dials)
sleep 2.3
printf '%s\n' '{"text":"hello three","user":"u","chat":"c1","cursor":3}' >> "$W/run/channels/inbox/inbox.txt"; sleep 1.2
D3=$(dials); OK=$(delivered)
sleep 0.5
D4=$(dials)

echo "--- runs the daemon started ---"
PII_RUNTIME_DIR="$W/run" "$PII" run ls 2>/dev/null || echo "(run ls unavailable)"
kill -9 "$PID" 2>/dev/null; kill -9 "$FAKE" 2>/dev/null

echo "dials after run1=$D1 | after run2 (inside the 1 s window)=$D2 | after run3 (+1.4 s)=$D3 | settled=$D4 | delivered=$OK"
echo "--- the daemon's own refusal lines ---"
python3 - "$W/event.log" <<'PY'
import json, sys
seen = []
for line in open(sys.argv[1], errors="replace"):
    try: d = json.loads(line)
    except Exception: continue
    if d.get("kind") in ("channel.post_refused", "channel.posted"):
        f = d.get("fields") or {}
        seen.append(f"{d['kind']}: {(f.get('reason') or f.get('destination') or '')[:120]}")
for s in seen[:6]:
    print(" ", s)
print("  (total", len(seen), "post events)")
PY
echo
if [ "$D1" = "1" ] && [ "$D2" = "1" ] && [ "$D3" = "2" ] && [ "$D4" = "2" ] && [ "$OK" = "1" ]; then
  echo "E6 PASS — inside the provider's window nothing dialled; after it the post landed once"
elif [ "$D2" -gt 1 ]; then
  echo "E6 FAIL — run 2 dialled again inside the 1 s window (dial counter $D1 -> $D2): nothing paced the send"
else
  echo "E6 FAIL — dials $D1/$D2/$D3 settled=$D4 delivered=$OK (expected 1/1/2/2/1)"
fi
echo "scratch kept at $W (injected fault, never repaired by the harness)"
