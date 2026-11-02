#!/usr/bin/env python3
"""mqtt-devices.py — list what is actually on the broker, from the devices' own discovery messages.

Tasmota publishes `tasmota/discovery/<mac>/config` (retained) with the device's name, IP, module and firmware, and
`tele/<topic>/LWT` carries Online/Offline. This reads both and prints one row per device. Read-only.

Usage: mqtt-devices.py [--host 127.0.0.1] [--port 1883]
"""
import argparse
import json
import subprocess
import sys


def capture(topic, seconds=8):
    out = subprocess.run(
        ["timeout", str(seconds), "mosquitto_sub", "-h", HOST, "-p", str(PORT), "-t", topic, "-v"],
        capture_output=True, text=True, check=False,
    ).stdout
    rows = {}
    for line in out.splitlines():
        parts = line.split(" ", 1)
        if len(parts) == 2:
            rows.setdefault(parts[0], []).append(parts[1])
    return rows


ap = argparse.ArgumentParser()
ap.add_argument("--host", default="127.0.0.1")
ap.add_argument("--port", type=int, default=1883)
a = ap.parse_args()
HOST, PORT = a.host, a.port

disc = capture("tasmota/discovery/#")
lwt = capture("tele/+/LWT")

devices = {}
for topic, payloads in disc.items():
    if not topic.endswith("/config"):
        continue
    for p in payloads:
        try:
            d = json.loads(p)
        except Exception:
            continue
        mac = topic.split("/")[2]
        fns = [f for f in (d.get("fn") or []) if f]
        devices[mac] = {
            "name": d.get("dn") or (fns[0] if fns else "?"),
            "ip": d.get("ip", "?"),
            "module": d.get("md", "?"),
            "firmware": d.get("sw", "?"),
            "functions": ", ".join(fns[:4]),
        }

online = {}
for topic, payloads in lwt.items():
    dev = topic.split("/")[1] if "/" in topic else topic
    online[dev] = payloads[-1].strip()

if not devices:
    print("no Tasmota discovery messages found on the broker")
    sys.exit(0)

print(f"{len(devices)} device(s) on {HOST}:{PORT}\n")
for mac, d in sorted(devices.items(), key=lambda kv: kv[1]["name"].lower()):
    topic_name = d["name"]
    state = online.get(f"tasmota_{mac[-6:]}") or online.get(topic_name) or "unknown"
    print(f"  {d['name']:<12} {d['ip']:<14} {state:<9} {d['module']:<12} fw {d['firmware']}")
    if d["functions"]:
        print(f"  {'':12} functions: {d['functions']}")
