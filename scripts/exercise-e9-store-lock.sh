#!/usr/bin/env bash
# exercise-e9-store-lock.sh — E9: the store cannot be opened; pii must refuse loudly, not hang.
# Inject: hold the store's exclusive lock (redb takes one) BEFORE the daemon starts, prove the
# hold with a marker file, and only then start the daemon. The harness only observes.
# Verdict rule: if the marker never appears, the inject did not fire — ABORT, no verdict at all.
# Run: bash scripts/exercise-e9-store-lock.sh
set -uo pipefail
cd /projects/pi-improved || exit 1
. scripts/autonomy/env.sh 2>/dev/null
PII=/root/.local/bin/pii
W=$(mktemp -d /projects/_tmp/ex19e9-XXXXXX) || exit 1
mkdir -p "$W/run"
touch "$W/store.redb"
cat > "$W/pii.toml" <<CFG
[core]
model = "openai:glm-5.3-flash"
CFG
echo "scratch: $W"

# inject: hold the store file's exclusive lock; write the marker ONLY after acquiring it
python3 - "$W/store.redb" "$W/lock.pid" "$W/holding" <<'PY' &
import sys, os, fcntl, time
f = open(sys.argv[1], 'rb')
fcntl.flock(f, fcntl.LOCK_EX | fcntl.LOCK_NB)
open(sys.argv[2], 'w').write(str(os.getpid()))
open(sys.argv[3], 'w').close()
time.sleep(120)
PY
HOLDER=$!

# gate: the inject must PROVE itself before the daemon may be asked anything
for i in $(seq 1 20); do [ -f "$W/holding" ] && break; sleep 0.25; done
if [ ! -f "$W/holding" ]; then
  echo "ABORT: the holder never acquired the lock (holding marker absent) — no verdict is reported."
  kill -9 $HOLDER 2>/dev/null
  exit 3
fi
echo "inject fired: holder pid=$(cat "$W/lock.pid") holds $W/store.redb"

set -a; . /root/.config/pii/pii.env 2>/dev/null; set +a
PII_RUNTIME_DIR="$W/run" "$PII" daemon --store "$W/store.redb" --workspace "$W" \
  --config "$W/pii.toml" --log-file "$W/event.log" --foreground >> "$W/daemon.log" 2>&1 &
PID=$!
sleep 6
UP=0
PII_RUNTIME_DIR="$W/run" "$PII" health --json >/dev/null 2>&1 && UP=1
echo "daemon pid=$PID up=$UP"
kill -9 $PID 2>/dev/null
kill -9 $HOLDER 2>/dev/null
echo "--- daemon.log (the refusal must name the store) ---"
cut -c1-260 "$W/daemon.log"
echo "--- event.log (store lines) ---"
grep -i "store" "$W/event.log" 2>/dev/null | head -4 | cut -c1-260
echo "SUMMARY: E9_up=$UP (expected 0 — a locked store must be a loud refusal, not a running daemon)"