#!/usr/bin/env bash
# F5.3: SIGTERM to a daemon with an active run must exit 0 within 2s and
# remove the control socket. Writes only under _tmp/phase3/. A stale scratch
# dir is moved aside, never removed.
set -uo pipefail
export PATH=/usr/local/cargo/bin:$PATH
cd "$(dirname "$0")/../.."

PII=./target/debug/pii
T=_tmp/phase3/shutdown
if [ -d "$T" ]; then
    mv "$T" "$T.old-$(date +%s)"
fi
mkdir -p "$T/graphs/starter"
cp graphs/starter/slow.toml "$T/graphs/starter/slow.toml"

export PII_RUNTIME_DIR="$PWD/$T/rt"
export PII_MODEL="scripted:fixtures/phase3/model/assistant.json"

"$PII" daemon --sandbox off --store "$PWD/$T/store.redb" >"$T/daemon.log" 2>&1 &
DPID=$!
mkdir -p "$PII_RUNTIME_DIR/graphs/starter"
cp graphs/starter/slow.toml "$PII_RUNTIME_DIR/graphs/starter/slow.toml"

for _ in $(seq 1 100); do
    [ -e "$PII_RUNTIME_DIR/control.sock" ] && break
    sleep 0.05
done
if [ ! -e "$PII_RUNTIME_DIR/control.sock" ]; then
    echo "FAIL: daemon did not bind control.sock" >&2
    kill "$DPID" 2>/dev/null || true
    exit 1
fi

START_OUT=$("$PII" run start starter/slow 2>&1)
if ! echo "$START_OUT" | grep -q run_id; then
    echo "FAIL: run start: $START_OUT" >&2
    kill "$DPID" 2>/dev/null || true
    exit 1
fi

sleep 0.3
T0=$(date +%s%N)
kill -TERM "$DPID"
EXITCODE=124
for _ in $(seq 1 60); do
    if ! kill -0 "$DPID" 2>/dev/null; then
        wait "$DPID"
        EXITCODE=$?
        break
    fi
    sleep 0.05
done
T1=$(date +%s%N)
ELAPSED_MS=$(((T1 - T0) / 1000000))

if [ "$EXITCODE" -ne 0 ]; then
    echo "FAIL: daemon exit code $EXITCODE (expected 0)" >&2
    exit 1
fi
if [ "$ELAPSED_MS" -ge 2000 ]; then
    echo "FAIL: shutdown took ${ELAPSED_MS}ms (expected < 2000ms)" >&2
    exit 1
fi
if [ -e "$PII_RUNTIME_DIR/control.sock" ]; then
    echo "FAIL: control.sock still present after shutdown" >&2
    exit 1
fi
echo "shutdown: OK (exit=$EXITCODE elapsed_ms=$ELAPSED_MS)"
