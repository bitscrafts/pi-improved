#!/usr/bin/env bash
# modules/devbox/envfile.sh — fill the devbox env file ON THE HOST from the hermes env file, by NAME. No value is printed, logged or
# put on an argv: the remote side reads the source and writes the target (0600) itself; only names and lengths come back.
# Names carried: those in NAMES (default OLLAMA_API_KEY). ANTHROPIC auth is claude's login file, bound read-only by compose.yaml.
set -euo pipefail
HOST=${DEVBOX_HOST:-mvcorrea@10.10.10.217}
SRC=${DEVBOX_ENV_SRC:-/podman/assistants/hermes/root/.hermes/.env}
NAMES=${DEVBOX_ENV_NAMES:-OLLAMA_API_KEY}
ssh -o BatchMode=yes "$HOST" "SRC='$SRC' NAMES='$NAMES' bash -s" <<'REMOTE'
set -eu
umask 077
T=$HOME/.config/pii/devbox.env
: > "$T.new"
for n in $NAMES; do
  line=$(grep -E "^(export +)?$n=" "$SRC" | tail -1 || true)
  [ -n "$line" ] || { echo "MISSING $n in source"; continue; }
  v=${line#*=}; v=${v%\"}; v=${v#\"}; v=${v%\'}; v=${v#\'}
  printf '%s=%s\n' "$n" "$v" >> "$T.new"
  echo "carried $n (len=${#v})"
done
mv -f "$T.new" "$T"
stat -c '%a %U %s bytes %n' "$T"
REMOTE
