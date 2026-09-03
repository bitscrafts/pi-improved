#!/usr/bin/env bash
# E7: the ndjson emitted over loopback and over UDS must be byte-identical
# after normalising run_id and any *_ms field (F5.8). Writes only under
# _tmp/phase2/. A stale scratch dir is moved aside, never removed.
set -euo pipefail
export PATH=/usr/local/cargo/bin:$PATH
cd "$(dirname "$0")/../.."

FIXTURE=fixtures/phase2/hello.json
T=_tmp/phase2/e7
if [ -d "$T" ]; then
    mv "$T" "$T.old-$(date +%s)"
fi
mkdir -p "$T/loop" "$T/uds"

# --- loopback leg (no daemon reachable in a fresh runtime dir) -------------
PII_RUNTIME_DIR="$PWD/$T/loop" PII_MODEL="scripted:$FIXTURE" \
    ./target/debug/pii -p "hello" --json > "$T/loop.ndjson"
test -e "$T/loop/control.sock" && { echo "FAIL: loopback created a socket" >&2; exit 1; }

# --- UDS leg (live daemon, client connects over the control socket) --------
PII_RUNTIME_DIR="$PWD/$T/uds" PII_MODEL="scripted:$FIXTURE" \
    ./target/debug/pii daemon --sandbox off > "$T/daemon.log" 2>&1 &
DPID=$!
for _ in $(seq 1 100); do
    [ -e "$T/uds/control.sock" ] && break
    sleep 0.05
done
test -e "$T/uds/control.sock" || { echo "FAIL: daemon did not bind control.sock" >&2; kill "$DPID" 2>/dev/null; exit 1; }
PII_MODEL="scripted:$FIXTURE" \
    ./target/debug/pii -p "hello" --json --connect "$PWD/$T/uds/control.sock" > "$T/uds.ndjson"
kill "$DPID" 2>/dev/null || true
wait "$DPID" 2>/dev/null || true

# --- normalise and compare --------------------------------------------------
norm() {
    sed -E 's/"run_id":"r[0-9]+"/"run_id":"R"/g; s/"[a-z_]+_ms":[0-9]+/"x_ms":0/g' "$1"
}
diff <(norm "$T/loop.ndjson") <(norm "$T/uds.ndjson") \
    || { echo "FAIL: loopback and UDS ndjson differ" >&2; exit 1; }
grep -q '"kind":"run_finished"' "$T/loop.ndjson"
echo "uds-vs-loopback: OK"
