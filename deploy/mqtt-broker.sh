#!/usr/bin/env bash
# mqtt-broker.sh — bring up the broker the Zigbee gateway already expects, so pii's MQTT path can be exercised.
#
# MEASURED 2026-11-02:
#   gateway  http://10.10.10.223 → Tasmota 15.2.0.4, ESP32-D0WD-V3, "zb-gw03" (a Zigbee bridge)
#            Status 6 says: MqttHost=10.10.10.217, MqttPort=1883, MqttClient=DVES_C76EA4, MqttUser=DVES_USER
#   10.10.10.217 IS lnx (interface wlx90de80814455) — so the device is pointed at the right machine
#   nothing listens on 1883: no broker on lnx, none in a container, no mosquitto process anywhere
#
# So the device's telemetry has been going nowhere. This starts mosquitto and proves the device connects, WITHOUT
# touching the device: its configuration is already correct.
set -uo pipefail
HOST=mvcorrea@lnx
sshx(){ timeout "${SSH_TIMEOUT:-300}" ssh -o BatchMode=yes "$HOST" "$@"; }

sshx 'set -e
  if command -v mosquitto >/dev/null; then
    echo "broker: already installed ($(mosquitto -h 2>&1 | head -1))"
  else
    echo "installing mosquitto…"
    sudo -n apt-get update -qq >/dev/null 2>&1 || true
    sudo -n DEBIAN_FRONTEND=noninteractive apt-get install -y -qq mosquitto mosquitto-clients >/dev/null 2>&1
    echo "installed: $(command -v mosquitto)"
  fi
  # LAN-only listening, anonymous allowed because the device carries a username and no password is set on it.
  # Written as its own drop-in so the packaged default config keeps working.
  sudo -n mkdir -p /etc/mosquitto/conf.d
  printf "%s\n" \
    "# Zigbee bridge (zb-gw03, Tasmota) publishes here; pii subscribes. LAN only." \
    "listener 1883 0.0.0.0" \
    "allow_anonymous true" \
    "persistence false" \
    | sudo -n tee /etc/mosquitto/conf.d/zigbee.conf >/dev/null
  sudo -n systemctl enable --now mosquitto >/dev/null 2>&1
  sudo -n systemctl restart mosquitto
  sleep 2
  echo "service : $(systemctl is-active mosquitto) / $(systemctl is-enabled mosquitto 2>/dev/null)"
  echo "listening: $(ss -ltnp 2>/dev/null | grep -c ":1883" || echo 0) socket(s) on 1883"
'
