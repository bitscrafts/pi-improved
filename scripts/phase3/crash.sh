#!/usr/bin/env bash
# F5.4: a node that panics must write <runtime>/crashes/<ms>.json with
# location + message while the daemon stays up and keeps answering
# sys.status. Writes only under _tmp/phase3/.
set -uo pipefail
export PATH=/usr/local/cargo/bin:$PATH
cd "$(dirname "$0")/../.."

PII=./target/debug/pii
T=_tmp/phase3/crash
if [ -d "$T" ]; then
    mv "$T" "$T.old-$(date +%s)"
fi
mkdir -p "$T/graphs/starter"
cp crates/pii-graph/tests/fixtures/crash.toml "$T/graphs/starter/crash.toml"

export PII_RUNTIME_DIR="$PWD/$T/rt"
export PII_MODEL="scripted:fixtures/phase3/model/assistant.json"
mkdir -p "$PII_RUNTIME_DIR/graphs/starter"
cp crates/pii-graph/tests/fixtures/crash.toml "$PII_RUNTIME_DIR/graphs/starter/crash.toml"

"$PII" daemon --sandbox off --store "$PWD/$T/store.redb" >"$T/daemon.log" 2>&1 &
DPID=$!
for _ in $(seq 1 100); do
    [ -e "$PII_RUNTIME_DIR/control.sock" ] && break
    sleep 0.05
done
if [ ! -e "$PII_RUNTIME_DIR/control.sock" ]; then
    echo "FAIL: daemon did not bind control.sock" >&2
    kill "$DPID" 2>/dev/null || true
    exit 1
fi

OUT=$("$PII" run start starter/crash 2>&1)
if ! echo "$OUT" | grep -q run_id; then
    echo "FAIL: run start: $OUT" >&2
    kill "$DPID" 2>/dev/null || true
    exit 1
fi

CRASH_FILE=""
for _ in $(seq 1 100); do
    CRASH_FILE=$(ls "$PII_RUNTIME_DIR"/crashes/*.json 2>/dev/null | head -1)
    [ -n "$CRASH_FILE" ] && break
    sleep 0.05
done
if [ -z "$CRASH_FILE" ]; then
    echo "FAIL: no crash file under $PII_RUNTIME_DIR/crashes/" >&2
    kill "$DPID" 2>/dev/null || true
    exit 1
fi
grep -q '"location"' "$CRASH_FILE" || { echo "FAIL: crash file lacks location" >&2; exit 1; }
grep -q '"message"' "$CRASH_FILE" || { echo "FAIL: crash file lacks message" >&2; exit 1; }

STATUS=$("$PII" sys status 2>&1)
if ! echo "$STATUS" | grep -q '"protocol"'; then
    echo "FAIL: daemon not answering sys.status after crash: $STATUS" >&2
    kill "$DPID" 2>/dev/null || true
    exit 1
fi

kill "$DPID" 2>/dev/null || true
wait "$DPID" 2>/dev/null || true
echo "crash: OK ($(basename "$CRASH_FILE"))"
