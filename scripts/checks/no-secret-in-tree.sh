#!/usr/bin/env bash
# Spec 157: the Discord token's value appears nowhere it must not: the repo tree, the generated unit,
# pii.toml, or the store. Prints file names only, never the value. Exit 0 = clean.
#
# The value comes from $PII_DISCORD_TOKEN, else from the PII_DISCORD_TOKEN= line of $CONFIG_DIR/pii.env.
# With neither, the value scan has nothing to look for and says so; the token-SHAPE scan still runs.
#
# Environment (all optional): CONFIG_DIR (default $HOME/.config/pii), STATE_DIR (default
# $HOME/.local/state/pii), UNIT_DIR (default $HOME/.config/systemd/user), RUNTIME_DIR (default /run/user/<uid>/pii).
set -uo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)" || exit 2
cd "$root" || exit 2
CONFIG_DIR="${CONFIG_DIR:-$HOME/.config/pii}"
STATE_DIR="${STATE_DIR:-$HOME/.local/state/pii}"
UNIT_DIR="${UNIT_DIR:-$HOME/.config/systemd/user}"
RUNTIME_DIR="${RUNTIME_DIR:-/run/user/$(id -u)/pii}"

tmp="$(mktemp -d)" || exit 2
trap 'rm -f "$tmp/value" "$tmp/files" "$tmp/stores"; rmdir "$tmp"' EXIT

value="${PII_DISCORD_TOKEN:-}"
if [ -z "$value" ] && [ -r "$CONFIG_DIR/pii.env" ]; then
  value="$(sed -n 's/^PII_DISCORD_TOKEN=//p' "$CONFIG_DIR/pii.env" | head -1)"
fi

# What to scan: the tree (tracked + untracked, not ignored), the unit, the config, the stores.
git ls-files -co --exclude-standard -z >"$tmp/files" 2>/dev/null
: >"$tmp/stores"
for f in "$UNIT_DIR/pii.service" /etc/systemd/system/pii.service "$CONFIG_DIR/pii.toml" \
  "$STATE_DIR/store.redb" "$RUNTIME_DIR/channel.redb" "$RUNTIME_DIR/timer.redb"; do
  [ -f "$f" ] && printf '%s\0' "$f" >>"$tmp/stores"
done

hits=0
if [ -n "$value" ]; then
  printf '%s\n' "$value" >"$tmp/value"   # -f keeps the value out of argv
  for list in files stores; do
    while IFS= read -r -d '' f; do
      if grep -aFq -f "$tmp/value" -- "$f" 2>/dev/null; then
        echo "LEAK: token value found in $f"
        hits=$((hits + 1))
      fi
    done <"$tmp/$list"
  done
  echo "value scan: done ($(tr -cd '\0' <"$tmp/files" | wc -c) tree files, $(tr -cd '\0' <"$tmp/stores" | wc -c) unit/config/store files)"
else
  echo "value scan: SKIPPED, no PII_DISCORD_TOKEN in the environment or $CONFIG_DIR/pii.env"
fi

# The shape of a Discord bot token, anywhere in the tree: a pasted real one is caught without knowing it.
shape='[MNO][A-Za-z0-9_-]{23,25}\.[A-Za-z0-9_-]{6}\.[A-Za-z0-9_-]{27,}'
while IFS= read -r -d '' f; do
  if grep -aEq -- "$shape" "$f" 2>/dev/null; then
    echo "LEAK: a Discord-token-shaped string in $f"
    hits=$((hits + 1))
  fi
done <"$tmp/files"

if [ "$hits" -ne 0 ]; then echo "FAIL: $hits file(s)"; exit 1; fi
echo "no secret in tree: ok"
