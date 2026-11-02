#!/usr/bin/env bash
# two-daemons.sh — land the topology the user asked for: PRODUCTION native on the host, DEVELOPMENT in the hermes
# container, and nothing else.
#
# The dev instance keeps the same binary and its own store, but it must NOT be armed for the digest: two daemons with
# the same cron entry and the same Discord channel would post the digest twice a day. So the dev config's [[cron]]
# block is commented out (the original is kept beside it), and production keeps it.
set -uo pipefail
HOST=mvcorrea@lnx
say(){ printf '%s\n' "$*"; }

say "=== 1. production (host, user unit) — confirm and record its env for the CLI"
timeout 60 ssh -o BatchMode=yes "$HOST" 'mkdir -p ~/.config/environment.d && printf "PII_RUNTIME_DIR=/run/user/1000/pii\n" > ~/.config/environment.d/pii.conf && systemctl --user is-active pii.service; PII_RUNTIME_DIR=/run/user/1000/pii ~/.local/bin/pii health 2>&1 | head -2' 2>&1 | tail -3

say "=== 2. development (container) — disarm the digest so it cannot double-post"
cd /projects/pi-improved || exit 1
CFG=/root/.config/pii/pii.toml
if grep -q '^\[\[cron\]\]' "$CFG"; then
  cp -p "$CFG" "$CFG.dev-backed-up-$(date -u +%Y%m%dT%H%M%SZ)"
  python3 - "$CFG" <<'PY'
import re, sys
p = sys.argv[1]; s = open(p).read()
# comment out the whole [[cron]] block, keeping it readable and reversible
out = re.sub(r'(?ms)^\[\[cron\]\].*?(?=^\[\[|\Z)', lambda m: ''.join('#' + l if l.strip() else l for l in m.group(0).splitlines(True)), s)
open(p, 'w').write(out)
print("cron blocks active now:", len(re.findall(r'(?m)^\[\[cron\]\]', out)))
PY
else
  say "no [[cron]] block in the dev config (already disarmed)"
fi

say "=== 3. start the dev daemon in the container, with its own runtime dir"
ARGV="/root/.local/bin/pii daemon --foreground --config /root/.config/pii/pii.toml --store /root/.local/state/pii/store.redb --workspace /root/.local/state/pii --log-file /root/.local/state/pii/pii.log --log-rotate-bytes 10485760 --log-keep-files 5"
mkdir -p /run/user/0/pii
( set -a; . /root/.config/pii/pii.env 2>/dev/null; set +a; export PII_RUNTIME_DIR=/run/user/0/pii; cd /projects && exec $ARGV ) > /tmp/pii-dev.log 2>&1 &
sleep 6
PII_RUNTIME_DIR=/run/user/0/pii /root/.local/bin/pii health 2>&1 | head -2

say "=== 4. exactly two daemons, and no more"
say "container (dev):   $(pgrep -fc '/root/.local/bin/pii daemon' || echo 0) process(es)"
say "host (production): $(timeout 40 ssh -o BatchMode=yes "$HOST" 'pgrep -fc "/home/mvcorrea/.local/bin/pii daemon" || echo 0') process(es)"
say "--- host unit:"
timeout 40 ssh -o BatchMode=yes "$HOST" 'systemctl --user is-enabled pii.service; systemctl --user is-active pii.service; systemctl --user show pii.service -p MainPID --value' 2>&1 | tail -3
