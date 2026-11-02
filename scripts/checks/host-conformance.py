#!/usr/bin/env python3
"""host-conformance.py — the five things that are true of the container and false of production.

The gates in this repo run in the `hermes` container against the container's config. Production runs on the host
`lnx`, as a different user, from a different binary, with a different config file and a different installer — which
is why the defects Opus measured on 2026-11-02 are invisible to all fifteen gate rows:

  (a) the host config carried container paths (`/root/.local/...`), so every run logged trajectory.write.failed;
  (b) `channel.redb` and `timer.redb` sat on tmpfs (`/run/user/1000/pii`), so a reboot loses the cursor and timers;
  (c) plain `pii health` answered "no daemon answered", because the client and the unit disagreed on the socket;
  (d) error-level `*.failed` events accumulate after a start and nothing reads them;
  (e) the runtime scan reports `daemon-start-not-ready` for starts that were healthy.

Two modes, one implementation of the five checks: `--fixture <file.json>` (the tests drive this) and a host mode
over ssh (`PII_HOST`). An unreachable host REFUSES with exit 2 — a check that cannot run is not a check that passed.
"""
from __future__ import annotations

import json
import os
import re
import subprocess
import sys

CHECKS = ["config paths", "durable stores", "health from a plain shell", "no failures since the last start",
          "no false start alarm"]


def _read_fixture(path: str) -> dict:
    with open(path, encoding="utf-8") as fh:
        return json.load(fh)


def check_config(fx: dict) -> tuple[bool, str]:
    bad = []
    for n, line in enumerate(fx.get("config", "").splitlines(), 1):
        stripped = line.lstrip()
        if stripped.startswith("#"):
            continue
        if "/root" in line or "/projects" in line:
            bad.append(f"{n}:{stripped}")
    if bad:
        return False, f"container paths in the host config: {bad[0]}"
    return True, "no /root or /projects path outside a comment"


STORE_RE = re.compile(r"^\s+[a-z_]+:\s+(/\S+)")


def stores_in(fx: dict) -> list[str]:
    """Every store path the daemon reports, from the TEXT `pii health` prints, plus any .redb still in the runtime
    directory. One parser for both modes: the host and the fixture produce the same text."""
    found = [m.group(1) for m in (STORE_RE.match(ln) for ln in fx.get("health_text", "").splitlines()) if m]
    for ln in fx.get("runtime_ls", "").splitlines():
        ln = ln.strip()
        if ln.endswith(".redb"):
            found.append(ln)
    return found


def check_stores(fx: dict) -> tuple[bool, str]:
    stores = stores_in(fx)
    if not stores:
        # nothing parsed is NOT a pass: the check would then be silent about the machine it cannot see.
        return False, "no store path could be read (health printed nothing usable, and no .redb in the runtime dir)"
    bad = [s for s in stores if s.startswith("/run/")]
    if bad:
        return False, f"durable store on a tmpfs: {bad[0]}"
    return True, f"{len(stores)} store(s), none under /run/"


def check_health(fx: dict) -> tuple[bool, str]:
    hp = fx.get("health_plain", {})
    rc = int(hp.get("rc", 1))
    if rc == 0:
        return True, "pii health answers with no environment set"
    return False, f"plain `pii health` fails (rc {rc}): {hp.get('stderr', '').strip()[:60]}"


def check_failures(fx: dict) -> tuple[bool, str]:
    log = fx.get("log", [])
    # the last start is the baseline: only what came after it counts
    last_start = -1
    for i, ev in enumerate(log):
        if ev.get("kind") == "daemon.started":
            last_start = i
    failed = [ev for ev in log[last_start + 1:] if str(ev.get("kind", "")).endswith(".failed")]
    if failed:
        return False, f"{len(failed)} error event(s) since the last start, first: {failed[0].get('kind')}"
    return True, "no *.failed event since the last daemon.started"


def check_alarm(fx: dict) -> tuple[bool, str]:
    text = fx.get("incidents", "")
    if "usage: pii incidents" in text:
        # the command was refused for a missing argument, so this check has NO input: refusing is honest, passing is not
        return False, "the incident scan printed its usage (a missing --log), so there is nothing to read"
    hits = [ln for ln in text.splitlines() if "daemon-start-not-ready" in ln]
    if hits:
        return False, f"{len(hits)} false start alarm(s), first: {hits[0][:70]}"
    return True, "no daemon-start-not-ready for any start"


RUNNERS = [check_config, check_stores, check_health, check_failures, check_alarm]


def run(fx: dict) -> int:
    passed = failed = 0
    for name, fn in zip(CHECKS, RUNNERS):
        ok, why = fn(fx)
        print(f"{'PASS' if ok else 'FAIL'}  {name}: {why}")
        passed += ok
        failed += not ok
    print(f"HOST-CONFORMANCE: {passed} passed, {failed} failed")
    return 0 if failed == 0 else 1


def over_ssh(host: str, timeout: int) -> dict:
    """Gather the five inputs from a live host. Read-only: cat, grep, findmnt, pii health, pii incidents scan."""
    script = r"""
set -u
C=${PII_CONFIG:-$HOME/.local/pii/pii.toml}
echo '@@config'
cat "$C" 2>/dev/null
echo '@@health'
PII_RUNTIME_DIR=${PII_RUNTIME_DIR:-/run/user/$(id -u)/pii} "$HOME/.local/bin/pii" health 2>/dev/null
echo '@@runtime_ls'
ls -1 /run/user/$(id -u)/pii/*.redb 2>/dev/null
echo '@@health_plain'
env -u PII_RUNTIME_DIR "$HOME/.local/bin/pii" health >/dev/null 2>&1; echo "rc=$?"
echo '@@log'
tail -200 "${PII_LOG:-$HOME/.local/state/pii/pii.log}" 2>/dev/null
echo '@@incidents'
env -u PII_RUNTIME_DIR "$HOME/.local/bin/pii" incidents scan --source runtime --log "$HOME/.local/state/pii/pii.log" 2>&1 | tail -60
"""
    try:
        out = subprocess.run(["ssh", "-o", "BatchMode=yes", "-o", f"ConnectTimeout={timeout}", host, script],
                             capture_output=True, text=True, timeout=timeout + 20)
    except subprocess.TimeoutExpired:
        print("REFUSED: cannot reach the host (timeout)"); sys.exit(2)
    if out.returncode != 0:
        print(f"REFUSED: cannot reach {host} (ssh rc {out.returncode}): {out.stderr.strip()[:80]}"); sys.exit(2)
    parts = re.split(r"^@@(\w+)$", out.stdout, flags=re.M)
    sections = {parts[i]: parts[i + 1] for i in range(1, len(parts) - 1, 2)}
    rc_line = sections.get("health_plain", "").strip()
    log = []
    for line in sections.get("log", "").splitlines():
        line = line.strip()
        if line.startswith("{"):
            try:
                log.append(json.loads(line))
            except json.JSONDecodeError:
                pass
    return {"config": sections.get("config", ""),
            "health_text": sections.get("health", ""),
            "runtime_ls": sections.get("runtime_ls", ""),
            "health_plain": {"rc": int(rc_line.split("=")[-1] or 1) if rc_line else 1,
                             "stderr": "no daemon answered"},
            "log": log,
            "incidents": sections.get("incidents", "")}


def main(argv: list[str]) -> int:
    fixture = host = None
    timeout = int(os.environ.get("PII_SSH_TIMEOUT", "10"))
    for i, a in enumerate(argv):
        if a == "--fixture" and i + 1 < len(argv):
            fixture = argv[i + 1]
        if a == "--host" and i + 1 < len(argv):
            host = argv[i + 1]
    host = host or os.environ.get("PII_HOST")
    if fixture:
        return run(_read_fixture(fixture))
    if not host:
        print("REFUSED: no host — set PII_HOST or pass --fixture <file.json>")
        return 2
    return run(over_ssh(host, timeout))


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
