#!/usr/bin/env bash
# E13 (001 EC3): kill -9 a live worker mid-invoke; death must be observable
# and the restart (fresh child + successful hello) must complete < 2000 ms.
# Drives the REAL wire (pii worker-exec, pii.worker.v1 frames on stdio).
# Writes only under _tmp/phase3/. A stale scratch dir is moved aside.
set -euo pipefail
export PATH=/usr/local/cargo/bin:$PATH
cd "$(dirname "$0")/../.."

PII=./target/debug/pii
T=_tmp/phase3/worker-kill
if [ -d "$T" ]; then
    mv "$T" "$T.old-$(date +%s)"
fi
mkdir -p "$T"

start_worker() { # $1 = tag; emits one JSON line with the shell pid
    rm -f "$T/in" "$T/out"
    mkfifo "$T/in"
    "$PII" worker-exec --name w --tag "$1" < "$T/in" > "$T/out" &
    local shpid=$!
    exec 9>"$T/in" # hold the fifo open so the worker stays alive
    printf '{"kind":"req","id":1,"op":"hello","args":{}}\n' >&9
    # poll for the res line (head -n 1 can race the fifo buffering)
    local line=""
    for _ in $(seq 1 100); do
        line=$(head -n 1 "$T/out" 2>/dev/null || true)
        [ -n "$line" ] && break
        sleep 0.02
    done
    echo "$line" | grep -q '"kind":"res"' \
        || { echo "FAIL: no hello res: '$line'" >&2; exit 1; }
    echo "$shpid"
}

# --- live worker: in-flight sleep, then kill -9 (E13's exact form) ----------
start_worker a > "$T/pid0"
WPID=$(cat "$T/pid0")
kill -0 "$WPID" 2>/dev/null || { echo "FAIL: worker not alive" >&2; exit 1; }
printf '{"kind":"req","id":2,"op":"sleep","args":{"ms":3000}}\n' >&9
sleep 0.2
kill -9 "$WPID"
sleep 0.2
if kill -0 "$WPID" 2>/dev/null; then
    echo "FAIL: worker survived kill -9" >&2
    exit 1
fi
echo "worker_died: worker=w pid=$WPID code=killed"

# --- restart: death -> fresh child + hello, measured (took_ms < 2000) -------
T0=$(date +%s%N)
start_worker a > "$T/pid1"
NPID=$(cat "$T/pid1")
T1=$(date +%s%N)
TOOK_MS=$(( (T1 - T0) / 1000000 ))
[ "$TOOK_MS" -lt 2000 ] || { echo "FAIL: restart took ${TOOK_MS}ms" >&2; exit 1; }
[ "$NPID" != "$WPID" ] || { echo "FAIL: restart reused the dead pid" >&2; exit 1; }
echo "worker_restarted: worker=w pid=$NPID restarts=1 took_ms=$TOOK_MS"

# the restarted worker serves again
printf '{"kind":"req","id":2,"op":"echo","args":{"v":42}}\n' >&9
sleep 0.2
grep -q '"ok":42' "$T/out" || { echo "FAIL: restarted worker dead" >&2; exit 1; }

exec 9>&-
echo "worker-kill: OK (took_ms=$TOOK_MS < 2000)"