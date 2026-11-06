#!/usr/bin/env bash
# F6.8/N9/E20: with a Discord bot token configured, the token must appear in
# NO captured event, trace, status, or daemon log, and the Authorization
# header is never logged verbatim. Writes only under _tmp/phase3/.
set -uo pipefail
export PATH=/usr/local/cargo/bin:$PATH
cd "$(dirname "$0")/../.."

PII=./target/debug/pii
T=_tmp/phase3/secret-scan
TOKEN=SUPERSECRETTOKEN12345
if [ -d "$T" ]; then
    mv "$T" "$T.old-$(date +%s)"
fi
mkdir -p "$T"

export PII_RUNTIME_DIR="$PWD/$T/rt"
export PII_MODEL="scripted:fixtures/phase3/model/assistant.json"
export PII_DISCORD_TOKEN="$TOKEN"
export PII_DISCORD_CHANNEL_ID=123456789
export PII_DISCORD_API_BASE="http://127.0.0.1:1" # unreachable: never contacted

mkdir -p "$PII_RUNTIME_DIR/graphs/starter"
cp crates/pii-graph/tests/fixtures/slow.toml "$PII_RUNTIME_DIR/graphs/starter/slow.toml"

"$PII" daemon --sandbox off --store "$PWD/$T/store.redb" >"$T/daemon.log" 2>&1 &
DPID=$!
for _ in $(seq 1 100); do
    [ -e "$PII_RUNTIME_DIR/control.sock" ] && break
    sleep 0.05
done
if [ ! -e "$PII_RUNTIME_DIR/control.sock" ]; then
    echo "FAIL: daemon did not bind" >&2
    kill "$DPID" 2>/dev/null || true
    exit 1
fi

# Capture every wire surface the token must never reach.
"$PII" -p "hello" --json >"$T/events.ndjson" 2>&1 || true
"$PII" sys status >"$T/status.json" 2>&1 || true
RUN_ID=$("$PII" run list 2>/dev/null | sed -n 's/.*"run_id":"\([^"]*\)".*/\1/p' | head -1)
if [ -n "$RUN_ID" ]; then
    "$PII" run trace "$RUN_ID" >"$T/trace.ndjson" 2>&1 || true
fi

for f in "$T"/events.ndjson "$T"/status.json "$T"/trace.ndjson "$T"/daemon.log; do
    if grep -q "$TOKEN" "$f" 2>/dev/null; then
        echo "FAIL: token leaked into $f" >&2
        kill "$DPID" 2>/dev/null || true
        exit 1
    fi
done
if grep -q "Authorization: Bot $TOKEN" "$T/daemon.log"; then
    echo "FAIL: Authorization header logged verbatim" >&2
    kill "$DPID" 2>/dev/null || true
    exit 1
fi

kill "$DPID" 2>/dev/null || true
wait "$DPID" 2>/dev/null || true
echo "secret-scan: OK (token absent from all captured surfaces)"
