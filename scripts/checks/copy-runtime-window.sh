#!/usr/bin/env bash
# Spec 346: copy the real runtime window the incidents come from — extracted, never hand-written.
#
# A fixture written by hand proves that the rule agrees with whoever wrote the fixture. This script copies EVERY
# line of the daemon's log that the window spans, so the rule is judged against what the daemon actually wrote.
#
# The window is computed over a fetched copy of the whole log (not by grepping on the host twice), which keeps the
# arithmetic in one place:
#   start = the last `daemon.started` at or before the FIRST event of any named run
#   end   = the first `daemon.started` at or after the LAST event of any named run
# so a run's post-decision events (a `channel.posted` after a `cron.disarmed`) are inside the window, which is the
# whole point: a window that stops before them cannot tell a stalled run from a finished one.
#
# Usage: copy-runtime-window.sh <host> <out.jsonl> --from RUN [--from RUN ...]
set -euo pipefail

HOST="${1:?usage: copy-runtime-window.sh <host> <out.jsonl> --from RUN [--from RUN ...]}"
OUT="${2:?usage: copy-runtime-window.sh <host> <out.jsonl> --from RUN [--from RUN ...]}"
shift 2
RUNS=()
while [ $# -gt 0 ]; do
  case "$1" in
    --from) RUNS+=("${2:?}"); shift 2 ;;
    *) echo "ABORT: unknown argument $1" >&2; exit 2 ;;
  esac
done
[ "${#RUNS[@]}" -gt 0 ] || { echo "ABORT: at least one --from <run id> is required" >&2; exit 2; }

TMP=$(mktemp "${TMPDIR:-/tmp}/pii-window-XXXXXX")
trap 'rm -f "$TMP"' EXIT
ssh "$HOST" 'cat $HOME/.local/state/pii/pii.log' > "$TMP"

python3 - "$TMP" "$OUT" "${RUNS[@]}" <<'PY'
import json, sys

log_path, out_path, runs = sys.argv[1], sys.argv[2], sys.argv[3:]
lines = open(log_path, encoding="utf-8", errors="replace").read().splitlines()

def kind(line):
    try:
        return json.loads(line).get("kind", "")
    except Exception:
        return ""

def mentions(line, run):
    return f'"run":"{run}"' in line

first = None
last = None
for i, line in enumerate(lines, start=1):
    if any(mentions(line, r) for r in runs):
        first = first or i
        last = i
if first is None:
    sys.exit(f"REFUSED: none of {runs} appears in the log")

start = first
for i in range(first, 0, -1):
    if kind(lines[i - 1]) == "daemon.started":
        start = i
        break

end = len(lines)
for i in range(last, len(lines) + 1):
    if kind(lines[i - 1]) == "daemon.started":
        end = i
        break

with open(out_path, "w", encoding="utf-8") as f:
    for line in lines[start - 1:end]:
        f.write(line + "\n")

counts = {r: sum(1 for l in lines[start - 1:end] if mentions(l, r)) for r in runs}
print(f"copied {out_path}: {end - start + 1} lines (log {start}-{end})")
for r, n in counts.items():
    print(f"  {r}: {n} event(s)")
PY
