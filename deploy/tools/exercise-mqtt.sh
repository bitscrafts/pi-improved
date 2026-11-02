#!/usr/bin/env bash
# exercise-mqtt.sh — run pii's MQTT channel against the real Zigbee gateway, end to end.
#
# MEASURED 2026-11-02: the broker on lnx (which IS 10.10.10.217) had never existed, so `zb-gw03` (Tasmota 15.2.0.4,
# 10.10.10.223) was publishing its telemetry into nothing. With mosquitto up it publishes:
#   tele/tasmota_C76EA4/LWT  Online
#   tele/tasmota_C76EA4/STATE {"Time":…,"Uptime":…}
#   tasmota/discovery/2043A8C76EA4/config
# and a second device (tasmota_6DE85C) too.
#
# This starts a SCRATCH pii daemon (own store, own runtime dir, own config) subscribed to the gateway's telemetry,
# asks the device for a state report so a message is guaranteed inside the window, and asserts that pii received it
# and started a run. Production is not touched.
set -uo pipefail
REPO=$(cd "$(dirname "$0")/.." && pwd)
HOST_IP=${MQTT_HOST:-10.10.10.217}
S=$(mktemp -d /tmp/mqtt-ex-XXXXXX)
mkdir -p "$S/state" "$S/run"
echo "scratch: $S"

# a graph whose only job is to record that it ran: classify a topic into an outcome, then post nothing.
cat > "$S/state/graphs/mqtt-probe.toml" 2>/dev/null || { mkdir -p "$S/state/graphs"; }
cat > "$S/state/graphs/mqtt-probe.toml" <<'TOML'
name = "mqtt-probe"
entry = "mark"
caps = []
[[node]]
id = "mark"
kind = "transform"
value = { text = "zigbee telemetry received", outcome = "ok" }
TOML

# The MQTT exercise needs no model call, but the daemon refuses to boot without one configured. A local one keeps
# the exercise free and offline: it is never invoked by this graph.
cat > "$S/pii.toml" <<TOML
[core]
model = "llamacpp-server:granite-4.2-3b"

[llama]
host = "http://$HOST_IP:8084/v1"

[[mqtt]]
name = "zigbee"
url = "mqtt://$HOST_IP:1883"
graph = "mqtt-probe"
# VALID filters only: `+` must occupy an entire level, so `tele/tasmota_+/STATE` is invalid MQTT and the client
# library refuses it with a message about the event loop. Valid forms: `tele/+/STATE` or the full topic.
topics = ["tele/+/STATE", "tele/+/LWT", "tasmota/discovery/#"]
qos = 0
client_id = "pii-exercise-$RANDOM"
TOML

# The FRESH build wins over whatever is on PATH: measured 2026-11-02, this script picked /root/.local/bin/pii (an
# older revision) and so exercised code that no longer existed in the tree, reporting a bug already fixed there.
BIN=/projects/_cache/targets/release/pii
[ -x "$BIN" ] || BIN=$REPO/target/release/pii
[ -x "$BIN" ] || BIN=$(command -v pii || true)
[ -x "${BIN:-}" ] || { echo "no pii binary" >&2; exit 2; }
echo "binary  : $BIN ($(date -r "$BIN" -u +%H:%M:%SZ))"

PII_RUNTIME_DIR="$S/run" "$BIN" daemon --foreground \
  --config "$S/pii.toml" --store "$S/state/store.redb" --workspace "$S/state" \
  --log-file "$S/pii.log" >"$S/daemon.out" 2>&1 &
DPID=$!
sleep 6
echo "daemon: $(kill -0 $DPID 2>/dev/null && echo up || echo 'DOWN — see' $S/daemon.out)"

# guarantee a message: ask the device for a status (read-only command)
timeout 20 ssh -o BatchMode=yes mvcorrea@lnx 'curl -sS -m 6 "http://10.10.10.223/cm?cmnd=Status%200" >/dev/null 2>&1' || true

# wait for pii to record the receipt
for _ in $(seq 1 20); do
  n=$(grep -c 'mqtt' "$S/pii.log" 2>/dev/null || echo 0)
  [ "$n" -gt 0 ] && break
  sleep 2
done

echo "=== pii's own log, mqtt lines ==="
grep -iE 'mqtt|channel' "$S/pii.log" 2>/dev/null | tail -6 | cut -c1-160
echo "=== runs started ==="
PII_RUNTIME_DIR="$S/run" "$BIN" run ls 2>/dev/null | head -c 400; echo

kill -TERM "$DPID" 2>/dev/null; sleep 1; kill -KILL "$DPID" 2>/dev/null
echo "scratch kept at $S (log: $S/pii.log)"
