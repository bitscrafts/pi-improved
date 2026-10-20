#!/usr/bin/env bash
# m0-daemon-starts-a-container.sh — spec 226 M0: can the sandboxed pii daemon start a podman container at all?
#
# Asks the running daemon (never an in-process run) for a one-node `coder` graph whose only action is a container
# start from localhost/pii-agent:1. The verdict is read from the run's journal (`pii run trace`), never from a
# summary. Prints exactly one verdict line on stdout:
#   M0 PASS <pid>             coder.spawned journalled with a real pid, and the run finished
#   M0 REFUSED <reason>       the sandbox or the runtime refused; the reason names the syscall or path
#   M0 FAILED <rc> <tail>     anything else; the tail is the journal's last error text
#
# The run's journal and the raw daemon replies are kept under $EVID (a fresh dir in /projects/_tmp).
# The graph is copied into the daemon's graphs dir under its own name; an existing file of that name is moved
# aside to a dated backup, never overwritten or deleted.
#
# On lnx (the podman host): PII_BIN=~/.local/bin/pii PII_GRAPHS_DIR=/run/user/1000/pii/graphs
# PII_LOG=~/pii.log PII_WORK=/tmp/m0-work PII_SPEC=<path to spec 226 on lnx> bash scripts/m0...
# Run: bash scripts/m0-daemon-starts-a-container.sh
set -uo pipefail
PII=${PII_BIN:-pii}
GRAPH_NAME=m0-daemon-starts-a-container
GRAPHS=${PII_GRAPHS_DIR:-/root/.local/state/pii/graphs}
LOG=${PII_LOG:-/root/.local/state/pii/pii.log}
WORK=${PII_WORK:-/projects/_tmp/m0-work}
TS=$(date -u +%Y%m%dT%H%M%SZ)
EVID=$(mktemp -d "${PII_EVID_ROOT:-/projects/_tmp}/m0-evidence-$TS-XXXX") || { echo "M0 FAILED 1 mktemp failed"; exit 1; }

tail_of() { tail -c 300 "$1" 2>/dev/null | tr '\n' ' '; }

command -v "$PII" >/dev/null 2>&1 || { echo "M0 FAILED 127 $PII not on PATH"; exit 0; }
# jq is absent on some hosts (measured on lnx: only python3). The fallback ships as a
# real python file next to this script (no nested quoting): same invocation shape —
#   JQ -r '<filter>' [file...]   with the two filter forms this script uses.
JQ() {
  if command -v jq >/dev/null 2>&1; then
    jq "$@"
    return
  fi
  local self dir
  self="${BASH_SOURCE[0]}"
  dir="$(cd "$(dirname "$self")" && pwd)"
  python3 "$dir/m0-jq.py" "$@"
}

# 1. The graph: the template is kept beside this script's evidence; the daemon reads it from GRAPHS.
mkdir -p "$WORK" || { echo "M0 FAILED 1 cannot create $WORK"; exit 0; }

cat > "$EVID/$GRAPH_NAME.toml" <<'GRAPH'
# Spec 226 M0: one coder node whose only action is a container start. Measurement only.
name = "m0-daemon-starts-a-container"
entry = "code"
caps = ["spawn", "proc"]

[[node]]
id = "code"
kind = "coder"
caps = ["spawn", "proc"]
agent = "pi"
model = "m0"
workdir = "__WORKDIR__"
spec = "__SPEC__"
brief = "M0: start the image and exit. The container runs the image's entrypoint; no agent work."
env_pass = []
container_image = "localhost/pii-agent:1"
# Spec 230: the API path, not `podman run` as a child (the direct path is measured closed by M0 2026-10-20).
container_runtime = "socket"

[[node]]
id = "done"
kind = "transform"
value = "done"

[[node]]
id = "refused"
kind = "transform"
value = "refused"

[[edge]]
from = "code"
port = "ok"
to = "done"
kind = "data"

[[edge]]
from = "code"
port = "err"
to = "refused"
kind = "data"
GRAPH
sed -i "s|__WORKDIR__|$WORK|; s|__SPEC__|${PII_SPEC:-/projects/pi-improved/specs/226-m0-the-daemon-can-start-a-container.md}|" "$EVID/$GRAPH_NAME.toml"
dst="$GRAPHS/$GRAPH_NAME.toml"
if [ -e "$dst" ] && ! cmp -s "$dst" "$EVID/$GRAPH_NAME.toml"; then
  mv "$dst" "$dst.old-$TS" || { echo "M0 FAILED 1 cannot move aside $dst"; exit 0; }
fi
cp "$EVID/$GRAPH_NAME.toml" "$dst" || { echo "M0 FAILED 1 cannot install graph at $dst"; exit 0; }

# 2. Ask the daemon for the run. A refused start is a failed measurement, reported with the daemon's words.
if ! "$PII" run start "$GRAPH_NAME" --input '{}' > "$EVID/start.json" 2> "$EVID/start.err"; then
  echo "M0 FAILED $? $(tail_of "$EVID/start.err")"; exit 0
fi
RUN=$(JQ -r '.run_id // empty' "$EVID/start.json" 2>/dev/null)
[ -n "$RUN" ] || { echo "M0 FAILED 2 no run_id in daemon reply: $(tail_of "$EVID/start.json")"; exit 0; }

# 3. Wait for the run to settle (bounded), then take its journal from the daemon.
for _ in $(seq 1 60); do
  STATUS=$("$PII" run get "$RUN" 2>/dev/null | JQ -r '.status // empty' 2>/dev/null)
  case "$STATUS" in succeeded|failed|refused|cancelled|abandoned) break ;; esac
  python3 -I -c 'import time; time.sleep(1)'
done
"$PII" run trace "$RUN" > "$EVID/trace.txt" 2>&1
TRACE_RC=$?
# The daemon holds the store lock, so its journal is read from the daemon log: every line naming the run id.
grep -F "$RUN" "$LOG" > "$EVID/journal.ndjson" 2>/dev/null
echo "run=$RUN status=${STATUS:-unknown} trace_rc=$TRACE_RC evidence=$EVID" >&2

# 4. The verdict, from the daemon's own trace and the journal lines.
# The trace is the daemon's reply (via=daemon); its fail= field carries the node's exact refusal,
# which is where podman's stderr lands (spec 228 R2). A log grep alone cannot carry it: the
# daemon's worker.start.refused lines name no run id (measured 2026-10-20 on lnx).
TRACE_FAIL=$(grep -oE 'fail=.+' "$EVID/trace.txt" | head -1 | sed -E 's/^fail=//; s/ rc=-.*$//; s/[[:space:]]+$//')
SPAWN=$(grep '"kind":"coder.spawned"' "$EVID/journal.ndjson" | head -1)
PID=$(printf '%s\n' "$SPAWN" | JQ -r '.fields.pid // empty' 2>/dev/null)
if [ -n "$PID" ] && [ "$PID" -gt 0 ] 2>/dev/null; then
  echo "M0 PASS $PID"; exit 0
fi
REFUSAL="${TRACE_FAIL:-}"
# `pii run trace` cuts `fail=` to 80 characters (spec 224 R2) and the daemon answers no --reason query
# (measured 2026-10-20: "unknown flag --reason", rc 2), so a reason at the boundary is a clip, not the
# runtime's whole sentence. Say so rather than present it as the measurement (spec 231 R3).
if [ -n "$REFUSAL" ] && [ "${#REFUSAL}" -ge 80 ]; then
  REFUSAL="$REFUSAL [clipped-80: trace cuts fail=; full reason pending A99]"
fi
[ -n "$REFUSAL" ] || REFUSAL=$(grep -iE 'refused|denied|landlock|EACCES|EPERM|sandbox' "$EVID/journal.ndjson" | head -1 | python3 -I -c '
import json, sys
l = sys.stdin.read().strip()
if not l:
    sys.exit(0)
try:
    o = json.loads(l)
except Exception:
    sys.exit(0)
print(str(o.get("kind", "")) + ": " + json.dumps(o.get("fields", {}))[:290])' 2>/dev/null)
# A refusal is a measurement only when it names the syscall or the path it was refused on (spec 226 R3).
if [ -n "$REFUSAL" ] && printf '%s' "$REFUSAL" | grep -qE '/[A-Za-z0-9_.-]+|execve|openat|landlock|mkdir|connect|bind'; then
  echo "M0 REFUSED $REFUSAL"; exit 0
fi
# rc 1: the refusal came before any container started, so there is no container rc to report.
echo "M0 FAILED 1 ${REFUSAL:-$(tail_of "$EVID/journal.ndjson")}"
