#!/usr/bin/env bash
# Spec 142: exercise deploy/install-pii.sh and deploy/rollback-pii.sh in a scratch prefix.
# Last line is "DRY-RUN OK" only if every assertion held. Nothing outside the scratch dir is touched.
set -euo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
mkdir -p /projects/_tmp/prefix
S="$(mktemp -d /projects/_tmp/prefix/dryrun.XXXXXX)"
fail() { echo "FAIL: $*" >&2; exit 1; }

# a fake llama.cpp server: GET /v1/models answers
mkdir -p "$S/web/v1"; echo '{"data":[]}' > "$S/web/v1/models"
PORT=$((20000 + RANDOM % 20000))
python3 -m http.server "$PORT" --bind 127.0.0.1 --directory "$S/web" >/dev/null 2>&1 &
SRV=$!
trap 'kill "$SRV" 2>/dev/null || true' EXIT
for _ in $(seq 1 50); do curl -sf "http://127.0.0.1:$PORT/v1/models" >/dev/null && break; sleep 0.1; done

# fake binaries: "old" already installed, "new" as the build output
mkdir -p "$S/bin" "$S/src"
for b in pii pii-web pii-evolve; do printf '#!/bin/sh\necho old-%s\n' "$b" > "$S/bin/$b"; chmod +x "$S/bin/$b"; done
for b in pii pii-web pii-evolve pii-mcp; do printf '#!/bin/sh\necho new-%s\n' "$b" > "$S/src/$b"; chmod +x "$S/src/$b"; done

export PREFIX="$S/bin" CONFIG_DIR="$S/cfg" STATE_DIR="$S/state" RUNTIME_DIR="$S/run" UNIT_DIR="$S/units"
export SKIP_BUILD=1 BIN_SRC="$S/src" SYSTEMCTL=true
export PII_DISCORD_TOKEN=dry-run-not-a-token PII_DISCORD_CHANNEL_ID=123456789

# 1. precondition failure: aborts before moving anything, names the host
if LLAMA_HOST=http://127.0.0.1:1/v1 bash "$REPO/deploy/install-pii.sh" >"$S/pre.out" 2>&1; then fail "unreachable llama host did not abort"; fi
grep -q '127.0.0.1:1/v1' "$S/pre.out" || fail "abort did not name the host"
[ "$("$S/bin/pii")" = old-pii ] || fail "precondition failure changed the installed binary"
ls "$S/bin".old-* >/dev/null 2>&1 && fail "precondition failure moved something aside"
[ ! -e "$S/cfg/pii.toml" ] || fail "precondition failure wrote config"

# 2. install
export LLAMA_HOST="http://127.0.0.1:$PORT/v1"
bash "$REPO/deploy/install-pii.sh" > "$S/install.out" 2>&1 || { cat "$S/install.out"; fail "install failed"; }
[ "$("$S/bin/pii")" = new-pii ] || fail "new pii not installed"
[ "$("$S/bin/pii-mcp.new")" = new-pii-mcp ] || fail "pii-mcp not staged"
[ ! -e "$S/bin/pii-mcp" ] || fail "pii-mcp was swapped (must be a separate step)"
ASIDE="$(ls -d "$S/bin".old-* | tail -1)"
[ "$("$ASIDE/pii")" = old-pii ] || fail "old pii not moved aside with a date"
[ -f "$S/units/pii.service" ] || fail "unit not written"
grep -q "ExecStart=$S/bin/pii daemon" "$S/units/pii.service" || fail "unit ExecStart wrong"
grep -q 'model = "llamacpp-server:granite-4.2-3b"' "$S/cfg/pii.toml" || fail "model missing"
grep -q "^host = \"$LLAMA_HOST\"" "$S/cfg/pii.toml" || fail "llama host missing"
grep -q '^trajectory_dir = ' "$S/cfg/pii.toml" || fail "trajectories not on"
[ "$(grep -c '^\[\[channel\]\]' "$S/cfg/pii.toml")" = 1 ] || fail "want exactly one channel"
[ "$(grep -c '^\[\[cron\]\]' "$S/cfg/pii.toml")" = 1 ] || fail "want exactly one cron"
grep -q 'discord-digest' "$S/cfg/pii.toml" || fail "digest cron missing"
grep -rq 'dry-run-not-a-token' "$S/cfg" "$S/units" && fail "token leaked into a written file"
for needle in 'pii.env' 'enable --now pii' 'health --json' 'rollback-pii.sh'; do
  grep -q -- "$needle" "$S/install.out" || fail "operator commands incomplete: $needle"
done

# 3. rollback
bash "$REPO/deploy/rollback-pii.sh" > "$S/rollback.out" 2>&1 || { cat "$S/rollback.out"; fail "rollback failed"; }
[ "$("$S/bin/pii")" = old-pii ] || fail "old pii does not answer after rollback"
[ ! -e "$ASIDE/pii" ] || fail "moved-aside copy still present (copied, not moved)"
grep -q 'RESTORED' "$S/rollback.out" || fail "rollback did not print what it restored"

echo "scratch: $S (left in place; remove by hand)"
echo "DRY-RUN OK"
