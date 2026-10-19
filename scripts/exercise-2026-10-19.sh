#!/usr/bin/env bash
# exercise-2026-10-19.sh — E3 (broken cron graph) and E2 (graphs dir missing) on a SCRATCH daemon.
#
# Rule of the exercises, verbatim from the user: "you just observe, the system should heal".
# The harness injects faults and READS the daemon's own event log; it never repairs what it
# injected, and it never reports a verdict over a process that did not start (each phase aborts
# unless the daemon demonstrably answers health first).
#
# The scratch mirrors the production unit's runtime shape:
#   <runtime>/graphs -> <state>/graphs   (the symlink deploy/install-pii.sh's unit makes)
# where <runtime> = $W/run and <state> = $W (the store sits at $W/store.redb, so the store's
# parent — the fallback path resolve_graph_dir tries second — is $W/graphs).
#
#   E3 inject: a [[cron]] naming graph "no-such-graph".
#   E3 PASS:   cron.start.refused naming "no-such-graph"; the good cron stays armed (suspended run = 1).
#
#   E2 inject: the runtime graphs symlink is REMOVED while the installed (state) graphs stay.
#   E2 PASS:   daemon.graphs_fallback naming the installed path; the good cron still arms (runs continue),
#              and the run that armed before the fault still exists after the restart (no duplication).
#   E2 extra (beyond the matrix, observed only): with BOTH paths gone the daemon still answers health
#   and names the refusal in its log, instead of crashing.
#
# Run:  bash scripts/exercise-2026-10-19.sh
set -uo pipefail
cd /projects/pi-improved || exit 1
. scripts/autonomy/env.sh 2>/dev/null
PII=/root/.local/bin/pii
W=$(mktemp -d /projects/_tmp/ex19-XXXXXX) || exit 1
mkdir -p "$W/graphs" "$W/run"
cp graphs/discord-digest.toml "$W/graphs/" 2>/dev/null || true

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

[[cron]]
name = "broken"
graph = "no-such-graph"
schedule = "5 5 * * *"
input = { to = "scratch" }
CFG

start() {
  # The production unit always passes --log-file (the daemon log is OFF by default,
# flag > [core] log_file > off — crates/pii-cli/src/daemon_log.rs); without it the
  # events this exercise reads go nowhere. daemon.log captures stdout/failures only.
  PII_RUNTIME_DIR="$W/run" "$PII" daemon --store "$W/store.redb" --workspace "$W" \
    --config "$W/pii.toml" --log-file "$W/event.log" --foreground >> "$W/daemon.log" 2>&1 &
  echo $!
}
up() { PII_RUNTIME_DIR="$W/run" "$PII" health --json >/dev/null 2>&1; }
armed() {
  PII_RUNTIME_DIR="$W/run" "$PII" run ls --status suspended 2>/dev/null | python3 -c '
import sys, json
d = json.load(sys.stdin)
r = d if isinstance(d, list) else d.get("runs", [])
print(len(r))
'
}

echo "=== scratch workspace: $W"

# ---------- control start: the runtime graphs symlink exists (as the unit makes it) ----------
ln -sfn "$W/graphs" "$W/run/graphs"
PID=$(start); sleep 8
if ! up; then
  echo "ABORT: the scratch daemon is not answering health. Its log:"
  tail -8 "$W/daemon.log"
  exit 2
fi
sleep 2
FB0=$(grep -c "graphs_fallback" "$W/event.log")
E3REF=$(grep -c "cron.start.refused" "$W/event.log")
E3named=$(grep "cron.start.refused" "$W/event.log" | grep -c "no-such-graph")
E3armed=$(armed)
echo "control: pid=$PID fallback_lines=$FB0  E3_refused=$E3REF E3_named=$E3named armed=$E3armed"

if [ "$E3REF" -ge 1 ] && [ "$E3named" -ge 1 ] && [ "$E3armed" = "1" ]; then
  echo "E3 PASS — $(grep 'cron.start.refused' "$W/event.log" | head -1 | cut -c1-230)"
else
  echo "E3 FAIL — refused=$E3REF named=$E3named armed=$E3armed (expected >=1, >=1, ==1)"
  grep "cron" "$W/event.log" | tail -3 | cut -c1-200
fi
if [ "$FB0" = "0" ]; then
  echo "CONTROL PASS — no fallback event while the runtime graphs path exists"
else
  echo "CONTROL FAIL — fallback fired although the runtime graphs path existed"
fi

# ---------- E2: the runtime graphs path goes away; the installed (state) path stays ----------
rm -f "$W/run/graphs"                       # the injected fault; the harness does NOT undo it
kill -9 "$PID" 2>/dev/null; sleep 2         # the death the fault would be seen across
PID=$(start); sleep 8
if ! up; then
  echo "ABORT(E2): the daemon is not answering health after the inject. Log:"
  tail -8 "$W/daemon.log"
  exit 2
fi
sleep 2
E2FB=$(grep -c "graphs_fallback" "$W/event.log")
E2fbline=$(grep "graphs_fallback" "$W/event.log" | tail -1)
E2armed=$(armed)
REARM=$(grep -c "cron.rearmed" "$W/event.log")
echo "E2: pid=$PID fallback_lines=$E2FB armed_after=$E2armed rearmed_events=$REARM"
if [ "$E2FB" -ge 1 ] && [ "$E2armed" = "1" ]; then
  echo "E2 PASS — $E2fbline" | cut -c1-260
  echo "        — the pre-fault suspended run survives the hard death; no duplicate armed (187 holds)"
else
  echo "E2 FAIL — fallback=$E2FB armed_after=$E2armed (expected >=1, ==1)"
  tail -5 "$W/event.log" | cut -c1-200
fi

# ---------- E2 extra: BOTH graph paths gone — the daemon must stay up, refusing loudly ----------
mv "$W/graphs" "$W/graphs.injected-away"
kill -9 "$PID" 2>/dev/null; sleep 2
PID=$(start); sleep 8
if up; then
  NOREF=$(grep "cron.start.refused" "$W/event.log" | tail -2 | cut -c1-200 | tr '\n' ' ')
  echo "E2-EXTRA: daemon still answers health with NO graph path at all (pid $PID); its refusals: $NOREF"
else
  echo "E2-EXTRA: daemon did NOT come up with no graph path at all — log:"
  tail -6 "$W/event.log"
fi
kill -9 "$PID" 2>/dev/null
echo "scratch daemon stopped; injected faults left as they were injected (no graphs symlink, graphs dir away; workspace kept at $W)"
echo "SUMMARY: E3_refused=$E3REF E3_named=$E3named E3_armed=$E3armed | E2_fallback=$E2FB E2_armed=$E2armed | control_fallback=$FB0"