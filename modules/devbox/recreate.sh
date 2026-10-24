#!/usr/bin/env bash
# Rebuild the devbox image (adds the login symlink), recreate the container with the directory login mount, restart the daemon, and PROVE the
# fix: (1) the file the container sees is the host-side one (same sha), (2) a capped haiku call inside it succeeds. Refuses if an agent is
# running inside the container (a recreate would kill it). Re-runnable.
set -uo pipefail
cd /projects/pi-improved || exit 2
HOST=mvcorrea@10.10.10.217
if ssh -o BatchMode=yes "$HOST" "podman exec pii-devbox sh -c 'pgrep -x claude || pgrep -x pi'" >/dev/null 2>&1; then echo "an agent runs in the container: refusing to recreate it"; exit 3; fi
bash modules/devbox/build.sh 2>&1 | tail -4
ssh -o BatchMode=yes "$HOST" 'bash -s' <<'REMOTE'
HP=/podman/assistants/hermes/projects/pi-improved
cd $HP/modules/devbox && podman-compose -f compose.yaml down 2>&1 | tail -1
DEVBOX_PROJECT=$HP DEVBOX_ENV_FILE=$HOME/.config/pii/devbox.env podman-compose -f compose.yaml up -d 2>&1 | tail -1
H=/podman/assistants/hermes/root/.claude/.credentials.json
echo "host     sha=$(sha256sum $H | cut -c1-12)"
podman exec pii-devbox sh -c 'echo "container sha=$(sha256sum /root/.claude/.credentials.json | cut -c1-12)"; ls -l /root/.claude/.credentials.json | cut -c1-90'
echo "== one capped call (haiku, \$0.05)"
podman exec -w /tmp pii-devbox sh -c 'claude --model claude-haiku-5-5 --permission-mode bypassPermissions --output-format json --max-budget-usd 0.05 -p "Reply PONG" > /tmp/p.json 2>&1; uv run --no-project python -c "
import json; d = json.load(open(\"/tmp/p.json\")); print(\"is_error=\", d.get(\"is_error\"), \"status=\", d.get(\"api_error_status\"), \"cost=\", d.get(\"total_cost_usd\"), \"result=\", repr(d.get(\"result\"))[:80])"'
REMOTE
cd /projects/pi-improved && bash modules/devbox/daemon.sh up 2>&1 | tail -2
