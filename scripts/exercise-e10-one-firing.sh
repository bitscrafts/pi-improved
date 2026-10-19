#!/usr/bin/env bash
# exercise-e10-one-firing.sh (v3) — E10: a cron set two minutes ahead fires EXACTLY ONE run, then silence.
#
# What v1/v2 got wrong (kept here, both measured):
#   v1 counted `run ls | grep -c run_id` — the digest cron's armed run is CONSUMED by its firing and a
#   NEW one re-arms ~1 s later, so the count can never move: structurally blind, false FAIL.
#   v2 pointed `[[cron]] schedule` at now+2 min and assumed firing follows it — but `schedule` is a
#   DISPLAY MIRROR; the wheel was armed for the GRAPH's timer-node cron (measured: in_ms=5606922 →
#   fire_at 09:00Z from the node cron `0 9 * * *`), so nothing fired on the injected minute at all.
#
# v3 injects the REAL knob: the graph file's timer node cron, set to the minute 2 minutes ahead.
# Observed with the store's own words (a run's events live in its store journal, not event.log):
#   before:   1 row, status suspended (armed)
#   +115 s:   exactly 2 rows — one TERMINAL (the firing; the discord post refuses on absent
#             credentials — the terminal is the signal, not its outcome) + one suspended (re-arm)
#   event.log shows ZERO `cron.rearmed` at start (v3 runs a daemon whose arm took the EXISTING run)
#             and EXACTLY ONE `cron.rearmed` after the firing — a second one inside the window is
#             the 178/183 bug class (re-arm before the next occurrence).
# Run: bash scripts/exercise-e10-one-firing.sh
set -uo pipefail
cd /projects/pi-improved || exit 1
. scripts/autonomy/env.sh 2>/dev/null
PII=/root/.local/bin/pii
W=$(mktemp -d /projects/_tmp/ex19e10-XXXXXX) || exit 1
mkdir -p "$W/graphs" "$W/run"
MIN=$(date -u -d '+2 minutes' '+%M')
HOUR=$(date -u -d '+2 minutes' '+%H')
DOM=$(date -u -d '+2 minutes' '+%d')
node-cron() { echo "$MIN $HOUR $DOM * *"; }
# the graph: the only edit is the timer node's cron (the real fire-time knob)
python3 - "$W" "$MIN" "$HOUR" "$DOM" graphs/discord-digest.toml <<'PY'
import sys, re
w, mi, hh, dom, src = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4], sys.argv[5]
g = open(src).read()
new_cron = f"{mi} {hh} {dom} * *"
g2, n = re.subn(r'cron = "[^"]*"', f'cron = "{new_cron}"', g, count=1)
assert n == 1, "the timer node's cron line was not found"
open(f"{w}/graphs/discord-digest.toml", "w").write(g2)
print("timer node cron injected:", new_cron)
PY
cat > "$W/pii.toml" <<CFG
[core]
model = "openai:glm-5.3-flash"

[[channel]]
name = "discord"
kind = "discord"
graph = "discord-digest"

[[cron]]
name = "digest"
graph = "discord-digest"
schedule = "0 12 * * *"
input = { to = "scratch" }
CFG
echo "scratch: $W (now $(date -u '+%H:%M:%SZ'), occurrence at $HOUR:$MIN UTC)"
setsid env PII_RUNTIME_DIR="$W/run" "$PII" daemon --store "$W/store.redb" --workspace "$W" \
  --config "$W/pii.toml" --log-file "$W/event.log" >> "$W/daemon.log" 2>&1 &
PID=$!
up() { PII_RUNTIME_DIR="$W/run" "$PII" health --json >/dev/null 2>&1; }
list() { PII_RUNTIME_DIR="$W/run" "$PII" run ls 2>/dev/null; }
statuses() { list | grep -o "\"status\":\"[a-z]*\"" | sort | uniq -c | tr '\n' ' '; echo; }
sleep 8
if ! up; then
  echo "ABORT: scratch daemon never answered health; daemon.log:"; tail -6 "$W/daemon.log"; kill -9 "$PID"; exit 2
fi
echo "t-2min: $(statuses)"
sleep 125   # occurrence + ~5 s
echo "occurrence+: $(statuses)"
sleep 40
echo "occurrence+45s: $(statuses)"
sleep 70
echo "occurrence+115s: $(statuses)"
# The verdict comes from ONE live read, counted as JSON ROWS — `run ls` returns one
# COMPACT LINE, so `grep -c run_id` counts lines, not rows (measured: rows=1 while the
# snapshot held 2 rows; that was the harness lying, not the system).
SNAP=$(list)
ROWS=$(printf '%s\n' "$SNAP" | python3 -c '
import sys, json
raw = sys.stdin.read().strip()
if not raw:
    print(0); raise SystemExit
d = json.loads(raw)
r = d if isinstance(d, list) else d.get("runs", [])
print(len(r))
')
TERM_ROW=$(printf '%s\n' "$SNAP" | grep -oc '"status":"succeeded"' || true)
TERM_ROW=$((TERM_ROW + $(printf '%s\n' "$SNAP" | grep -oc '"status":"failed"' || true)))
SUSP_ROW=$(printf '%s\n' "$SNAP" | grep -oc '"status":"suspended"' || true)
WHEEL=$(strings "$W/store.redb" 2>/dev/null | grep -o "timer:r[0-9]*:tick:1792[0-9]*" | sort -u | tr '\n' ' ')
kill -9 "$PID" 2>/dev/null
if [ "$ROWS" -eq 2 ] && [ "$TERM_ROW" -eq 1 ] && [ "$SUSP_ROW" -eq 1 ]; then
  echo "E10 PASS — rows=2 (one terminal + one re-armed); wheel keys: $WHEEL"
else
  echo "E10 FAIL — rows=$ROWS terminal=$TERM_ROW suspended=$SUSP_ROW (expected 2/1/1: one firing, one re-arm)"
fi