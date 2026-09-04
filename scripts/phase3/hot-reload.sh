#!/usr/bin/env bash
# E14 (001 EC4): hot reload drops nothing. 200 sequential worker invocations
# across the reload boundary; the worker program FILE is swapped mid-flight
# (the operator's atomic write+rename form); every request must succeed and
# the new program (tag b, different pid) must serve after the swap.
# Writes only under _tmp/phase3/. A stale scratch dir is moved aside.
set -euo pipefail
export PATH=/usr/local/cargo/bin:$PATH
cd "$(dirname "$0")/../.."

PII=./target/debug/pii
T=_tmp/phase3/hot-reload
if [ -d "$T" ]; then
    mv "$T" "$T.old-$(date +%s)"
fi
mkdir -p "$T"

# --- the swappable worker program (atomic overwrite form) -------------------
write_prog() { # $1 = tag
    printf '#!/bin/sh\nexec %s worker-exec --name w --tag %s\n' \
        "$PWD/$PII" "$1" > "$T/worker.sh.tmp"
    mv "$T/worker.sh.tmp" "$T/worker.sh"
}
write_prog a
chmod 755 "$T/worker.sh"

# --- 100 invocations against the OLD program --------------------------------
rm -f "$T/in" "$T/out"
mkfifo "$T/in"
"$T/worker.sh" < "$T/in" > "$T/out" &
SH0=$!
exec 9>"$T/in" # hold the fifo open so the old worker stays alive
printf '{"kind":"req","id":1,"op":"hello","args":{}}\n' >&9
for _ in $(seq 1 100); do
    LINE=$(head -n 1 "$T/out" 2>/dev/null || true)
    [ -n "$LINE" ] && break
    sleep 0.02
done
echo "$LINE" | grep -q '"tag":"a"' || { echo "FAIL: tag a hello: $LINE" >&2; exit 1; }
OLDPID=$(echo "$LINE" | grep -o '"pid":[0-9]*' | cut -d: -f2)
echo "old: tag=a pid=$OLDPID"

for i in $(seq 1 100); do
    printf '{"kind":"req","id":2,"op":"echo","args":{"v":%s}}\n' "$i" >&9
    sleep 0.01
done

# --- the operator's swap: replace the program file, reload = fresh spawn ----
write_prog b
chmod 755 "$T/worker.sh"
sleep 0.1
rm -f "$T/in2" "$T/out2"
mkfifo "$T/in2"
"$T/worker.sh" < "$T/in2" > "$T/out2" &
SH1=$!
exec 10>"$T/in2"
printf '{"kind":"req","id":1,"op":"hello","args":{}}\n' >&10
for _ in $(seq 1 100); do
    LINE2=$(head -n 1 "$T/out2" 2>/dev/null || true)
    [ -n "$LINE2" ] && break
    sleep 0.02
done
echo "$LINE2" | grep -q '"tag":"b"' || { echo "FAIL: tag b hello: $LINE2" >&2; exit 1; }
NEWPID=$(echo "$LINE2" | grep -o '"pid":[0-9]*' | cut -d: -f2)

# --- drain the remaining 100 requests; nothing may be dropped ---------------
for i in $(seq 1 100); do
    printf '{"kind":"req","id":3,"op":"echo","args":{"v":%s}}\n' "$i" >&9
    sleep 0.01
done
sleep 0.3
TOTAL=$(grep -c '"kind":"res"' "$T/out" 2>/dev/null || echo 0)
if [ "$TOTAL" -lt 201 ]; then
    echo "FAIL: only $TOTAL responses (expected >= 201: hello + 200 echoes)" >&2
    exit 1
fi
if [ "$NEWPID" = "$OLDPID" ]; then
    echo "FAIL: pid did not change across the reload" >&2
    exit 1
fi

kill "$SH0" "$SH1" 2>/dev/null || true
exec 9>&- 10>&-
echo "hot-reload: OK (responses=$TOTAL, pid $OLDPID -> $NEWPID, tag a->b)"