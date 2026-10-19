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
# Spec 197: the install refuses without a deploy card for the revision about to be built. Every install
# in this check — including the one whose llama host is unreachable — needs one, so the precondition that
# fires is the one the case is about. The card is planted in a scratch card dir, not the operator's.
export DEPLOY_CARD_DIR="$S/cards"; mkdir -p "$DEPLOY_CARD_DIR"
REV7="$(git -C "$REPO" rev-parse --short=7 HEAD)"; REV12="$(git -C "$REPO" rev-parse --short=12 HEAD)"
printf '{"note":"planted by deploy-dry-run.sh"}\n' > "$DEPLOY_CARD_DIR/card-$REV7.json"

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
grep -q 'dry-run-not-a-token' "$S/cfg/pii.toml" "$S/units/pii.service" && fail "token leaked into a written file"
[ -f "$S/cfg/pii.env" ] || fail "pii.env not written though both credentials were set"
[ "$(stat -c %a "$S/cfg/pii.env")" = 600 ] || fail "pii.env is not mode 0600"
grep -q '^PII_DISCORD_TOKEN=dry-run-not-a-token$' "$S/cfg/pii.env" || fail "pii.env lacks the token variable"
grep -q '^PII_DISCORD_CHANNEL_ID=123456789$' "$S/cfg/pii.env" || fail "pii.env lacks the channel id variable"
grep -q 'dry-run-not-a-token' "$S/install.out" && fail "install output printed the token"
for needle in 'pii.env' 'enable --now pii' 'health --json' 'rollback-pii.sh'; do
  grep -q -- "$needle" "$S/install.out" || fail "operator commands incomplete: $needle"
done

# 3. rollback
bash "$REPO/deploy/rollback-pii.sh" > "$S/rollback.out" 2>&1 || { cat "$S/rollback.out"; fail "rollback failed"; }
[ "$("$S/bin/pii")" = old-pii ] || fail "old pii does not answer after rollback"
[ ! -e "$ASIDE/pii" ] || fail "moved-aside copy still present (copied, not moved)"
grep -q 'RESTORED' "$S/rollback.out" || fail "rollback did not print what it restored"

# 4. spec 145: the graphs the config names are installed and placed on every unit start
GRAPHS=(discord-triage discord-digest)
for g in "${GRAPHS[@]}"; do   # R6.1
  cmp -s "$REPO/graphs/$g.toml" "$S/state/graphs/$g.toml" || fail "graph $g not installed byte-identical in STATE_DIR/graphs"
done
grep -q 'ExecStartPre=.*graphs' "$S/units/pii.service" || fail "unit has no ExecStartPre placing the graphs"
run_prestart() {   # R6.2: execute the unit's ExecStartPre lines in order
  local line
  while IFS= read -r line; do eval "${line#ExecStartPre=}"; done < <(grep '^ExecStartPre=' "$S/units/pii.service")
}
run_prestart || fail "ExecStartPre failed on an empty runtime dir"
for g in "${GRAPHS[@]}"; do
  [ -r "$S/run/graphs/$g.toml" ] || fail "graph $g not readable under RUNTIME_DIR/graphs after pre-start"
  cmp -s "$REPO/graphs/$g.toml" "$S/run/graphs/$g.toml" || fail "graph $g under RUNTIME_DIR/graphs differs from the repo's"
done
run_prestart || fail "second ExecStartPre run failed (not idempotent)"
[ -r "$S/run/graphs/discord-triage.toml" ] || fail "graphs unreadable after the second pre-start"
grep -q 'forum id is written into' "$S/cfg/pii.toml" || fail "pii.toml comment about the forum id is not the truth"

# R6.4: a second rollback has nothing to restore and must say so
if bash "$REPO/deploy/rollback-pii.sh" > "$S/rollback2.out" 2>&1; then fail "second rollback exited 0"; fi
grep -q 'NOTHING TO RESTORE' "$S/rollback2.out" || fail "second rollback did not print NOTHING TO RESTORE"

# R6.3: a re-install over an edited installed graph moves the edited copy aside
sleep 1   # the aside dir is named by epoch seconds
echo '# edited by the dry-run' >> "$S/state/graphs/discord-triage.toml"
bash "$REPO/deploy/install-pii.sh" > "$S/install2.out" 2>&1 || { cat "$S/install2.out"; fail "re-install failed"; }
ASIDE2="$(ls -d "$S/bin".old-* | tail -1)"
grep -q 'edited by the dry-run' "$ASIDE2"/*discord-triage.toml || fail "edited graph not moved into $ASIDE2"
cmp -s "$REPO/graphs/discord-triage.toml" "$S/state/graphs/discord-triage.toml" || fail "re-install did not restore the repo graph"

# 6. the operator's [core].model survives a re-install (measured 2026-10-19: an install regressed
#    the running daemon's model to the template default — the aside'd config said
#    openai:glm-5.3-flash per operator directive, the live config came back granite-4.2-3b).
#    The template's model is the FRESH-install default; a replaced install must carry it forward.
sed -i 's|^model = .*|model = "openai:glm-5.3-flash"   # operator choice|' "$S/cfg/pii.toml"
sleep 1
bash "$REPO/deploy/install-pii.sh" > "$S/install4.out" 2>&1 || { cat "$S/install4.out"; fail "re-install (model carry) failed"; }
ASIDE3="$(ls -d "$S/bin".old-* | tail -1)"
grep -q 'openai:glm-5.3-flash' "$ASIDE3/pii.toml" || fail "operator model not in the aside'd config before re-install"
grep -q '^model = "openai:glm-5.3-flash"' "$S/cfg/pii.toml" || fail "re-install regressed the operator's model"
grep -q 'carried forward by install-pii.sh' "$S/cfg/pii.toml" || fail "operator model carried without its provenance note"

# 5. spec 157: no credentials is a state, not an abort: install succeeds, writes no pii.env, no cron
sleep 1
S2="$S/nocreds"
if env -u PII_DISCORD_TOKEN -u PII_DISCORD_CHANNEL_ID CONFIG_DIR="$S2/cfg" STATE_DIR="$S2/state" UNIT_DIR="$S2/units" \
  bash "$REPO/deploy/install-pii.sh" > "$S/install3.out" 2>&1; then :; else cat "$S/install3.out"; fail "install without credentials aborted"; fi
[ ! -e "$S2/cfg/pii.env" ] || fail "pii.env written without credentials"
[ "$(grep -c '^\[\[cron\]\]' "$S2/cfg/pii.toml")" = 0 ] || fail "digest cron written without a channel id"
grep -q 'channels: \[\]' "$S/install3.out" || fail "install did not say the daemon will report no channels"

echo "scratch: $S (left in place; remove by hand)"
echo "DRY-RUN OK"
