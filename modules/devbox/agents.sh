#!/usr/bin/env bash
# modules/devbox/agents.sh — exercise the two agents INSIDE the devbox, on the host. Re-runnable; every step prints what it ran, the
# exit code and the facts that matter, never a credential. Spends real money only in steps 2-3 (claude haiku, one capped turn each,
# measured at ~$0.003 earlier) and uses ollama-cloud in step 5 (a free-tier route). Steps are independent; a failure does not stop the next.
#
# pi's config dir is the project-owned one (_tmp/pii-agent, mounted with the project), NEVER /root/.pi (operator rule).
set -uo pipefail
HOST=${DEVBOX_HOST:-mvcorrea@10.10.10.217}
HP=${DEVBOX_PROJECT:-/podman/assistants/hermes/projects/pi-improved}
ssh -o BatchMode=yes "$HOST" "DEVBOX_PROJECT=$HP DEVBOX_ENV_FILE=\$HOME/.config/pii/devbox.env bash -s" <<'REMOTE'
cd "$DEVBOX_PROJECT/modules/devbox" || exit 2
export DEVBOX_PROJECT DEVBOX_ENV_FILE
podman-compose -f compose.yaml up -d 2>&1 | tail -1
X() { podman exec -w /projects/pi-improved pii-devbox "$@"; }
echo "== 1. names only: what the container sees (no values)"
X sh -c 'for n in OLLAMA_API_KEY ANTHROPIC_API_KEY IS_SANDBOX; do eval v=\${$n:-}; if [ -n "$v" ]; then s="set(len=${#v})"; else s=unset; fi; printf "%s %s\n" $n "$s"; done; ls -l /root/.claude/.credentials.json | cut -c1-60; test -w /root/.claude/.credentials.json && echo "login file WRITABLE" || echo "login file read-only"; ls /root/.pi 2>&1 | head -1'
echo "== 2. claude + haiku, one turn, capped"
X sh -c 'cd /tmp && claude --model haiku --permission-mode bypassPermissions --output-format json --max-budget-usd 0.10 -p "Reply with exactly: PONG" 2>&1' | tr -d '\n' | sed -E 's/.*"result":"([^"]*)".*"total_cost_usd":([0-9.]+).*/result=\1 cost=\2/; s/(.{300}).*/\1/'; echo
echo "== 3. claude + haiku writes a file into a scratch dir inside the container"
X sh -c 'rm -rf /tmp/w && mkdir /tmp/w && cd /tmp/w && claude --model haiku --permission-mode bypassPermissions --output-format json --max-budget-usd 0.10 -p "Create a file hello.txt containing exactly PONG" >/tmp/w.out 2>&1; echo "rc=$? file=$(cat /tmp/w/hello.txt 2>&1)"; grep -o "\"total_cost_usd\":[0-9.]*" /tmp/w.out | head -1'
echo "== 4. pi starts and sees its own config dir (no model call)"
X sh -c 'PI_CODING_AGENT_DIR=/projects/pi-improved/_tmp/pii-agent pi --version; ls /projects/pi-improved/_tmp/pii-agent | tr "\n" " "; echo'
echo "== 5. pi + ollama-cloud, one tiny turn through the project-owned config dir"
X sh -c 'cd /tmp && rm -rf p && mkdir p && cd p && PI_CODING_AGENT_DIR=/projects/pi-improved/_tmp/pii-agent timeout 240 pi --provider ollama-cloud --model deepseek-v4.1-flash:cloud --mode json -p "Create a file pong.txt containing exactly PONG" >/tmp/p.out 2>&1; echo "rc=$? file=$(cat pong.txt 2>&1)"; grep -c . /tmp/p.out | sed "s/^/out_lines=/"; grep -a -o "\"modelId\":\"[^\"]*\"" /tmp/p.out | sort -u | head -2'
echo "== 6. scratch really left nothing in the project tree"
X sh -c 'echo "probe files in the project tree: $(git status --porcelain | grep -c -E "hello.txt|pong.txt")"'
REMOTE
