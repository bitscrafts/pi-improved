#!/bin/sh
# Launcher for a coding-agent worker node (spec 185, HARD RULE 0d): sets PATH and provider env INSIDE the
# script, then execs the agent argv it is given. Credentials come from the env file and are never echoed.
set -eu
PATH="/root/.local/bin:/usr/local/bin:/usr/bin:/bin:${PATH:-}"
export PATH
ENV_FILE="${PII_AGENT_ENV_FILE:-/root/.config/pii/agent.env}"
if [ -r "$ENV_FILE" ]; then
  set -a
  # shellcheck disable=SC1090
  . "$ENV_FILE"
  set +a
fi
[ "$#" -gt 0 ] || { echo "agent-worker.sh: no agent argv" >&2; exit 64; }
exec "$@"
