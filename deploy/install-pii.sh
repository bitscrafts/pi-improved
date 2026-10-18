#!/usr/bin/env bash
# Spec 142: install the pii release binaries, runtime config and systemd unit on THIS host.
# One command, idempotent. Never deletes: existing binaries/config are moved aside to a dated dir.
# It does NOT enable/start the unit. It writes $CONFIG_DIR/pii.env (mode 0600) from the operator's
# PII_DISCORD_TOKEN / PII_DISCORD_CHANNEL_ID when both are set; values go nowhere else and are never printed.
#
#   bash deploy/install-pii.sh
#
# Environment (all optional):
#   PREFIX        binary target dir            (default $HOME/.local/bin)
#   SCOPE         user | system                (default user)
#   LLAMA_HOST    llama.cpp server base URL    (default http://10.10.10.217:8082/v1)
#   CONFIG_DIR    pii.toml + pii.env location  (user: $HOME/.config/pii, system: /etc/pii)
#   STATE_DIR     store/log/trajectory root    (user: $HOME/.local/state/pii, system: /var/lib/pii)
#   RUNTIME_DIR   control socket dir           (user: /run/user/<uid>/pii, system: /run/pii)
#   UNIT_DIR      where the unit file goes     (user: $HOME/.config/systemd/user, system: /etc/systemd/system)
#   SKIP_BUILD=1  do not run cargo; install from BIN_SRC
#   BIN_SRC       built binaries dir           (default <repo>/target/release)
# Preconditions (checked before anything is moved): graphs/discord-{triage,digest}.toml exist, llama host answers, PREFIX is creatable.
# PII_DISCORD_TOKEN / PII_DISCORD_CHANNEL_ID are optional: without them no pii.env is written and the daemon
# starts with `channels: []` (spec 157).
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCOPE="${SCOPE:-user}"
LLAMA_HOST="${LLAMA_HOST:-http://10.10.10.217:8082/v1}"
PREFIX="${PREFIX:-$HOME/.local/bin}"
case "$SCOPE" in
  user)
    CONFIG_DIR="${CONFIG_DIR:-$HOME/.config/pii}"
    STATE_DIR="${STATE_DIR:-$HOME/.local/state/pii}"
    RUNTIME_DIR="${RUNTIME_DIR:-/run/user/$(id -u)/pii}"
    UNIT_DIR="${UNIT_DIR:-$HOME/.config/systemd/user}"
    CTL="systemctl --user" ;;
  system)
    CONFIG_DIR="${CONFIG_DIR:-/etc/pii}"
    STATE_DIR="${STATE_DIR:-/var/lib/pii}"
    RUNTIME_DIR="${RUNTIME_DIR:-/run/pii}"
    UNIT_DIR="${UNIT_DIR:-/etc/systemd/system}"
    CTL="systemctl" ;;
  *) echo "ABORT: SCOPE must be user or system, got '$SCOPE'" >&2; exit 2 ;;
esac
BIN_SRC="${BIN_SRC:-$REPO/target/release}"
BINS=(pii pii-web pii-evolve)   # swapped in place
STAGED=(pii-mcp)                # installed as <name>.new only: the pii-mcp swap is a separate step (runbook)
TS="$(date +%s)"

abort() { echo "ABORT (nothing was changed): $*" >&2; exit 1; }

# ---- R3: preconditions, before anything is built or moved ----
command -v curl >/dev/null || abort "curl not found"
curl -sf --max-time 5 "${LLAMA_HOST%/}/models" >/dev/null \
  || abort "llama.cpp host ${LLAMA_HOST} does not answer GET ${LLAMA_HOST%/}/models"
HAVE_CHANNEL=0
[ -n "${PII_DISCORD_TOKEN:-}" ] && [ -n "${PII_DISCORD_CHANNEL_ID:-}" ] && HAVE_CHANNEL=1
if [ ! -d "$PREFIX" ]; then
  mkdir -p "$PREFIX" 2>/dev/null || abort "target prefix $PREFIX does not exist and cannot be created"
fi
[ -w "$PREFIX" ] || abort "target prefix $PREFIX is not writable"
GRAPH_NAMES=(discord-triage discord-digest)   # the graphs the generated config names
for g in "${GRAPH_NAMES[@]}"; do
  [ -f "$REPO/graphs/$g.toml" ] || abort "graph '$g' is named by the generated config but $REPO/graphs/$g.toml is missing"
done

# ---- 1: build ----
if [ "${SKIP_BUILD:-0}" != 1 ]; then
  # Build from a CLEAN EXPORT of HEAD, never from the working tree.
  # Measured 2026-10-17 on three consecutive deploys: building in the repo shipped binaries that reported
  # "<rev>-dirty" whenever any path was uncommitted — including a worker's leftover — so the revision the deploy card
  # must quote exactly carried a suffix meaning "this is not the commit you verified". The gate already builds this way
  # (xtask/src/fresh_gate.rs: git archive); the installer now does too.
  EXPORT="$(mktemp -d "${TMPDIR:-/tmp}/pii-install-XXXXXX")" || abort "cannot create a clean export directory"
  git -C "$REPO" archive HEAD | tar -x -C "$EXPORT" || abort "git archive HEAD failed — is $REPO a git checkout?"
  # The export has no .git, so build.rs cannot read the revision from it: hand it in explicitly.
  # Measured 2026-10-18 — the first export-built binary reported "unknown" for a build correctly made from 136b23e.
  export PII_BUILD_REVISION="$(git -C "$REPO" rev-parse --short=12 HEAD)"
  (cd "$EXPORT" && cargo build --release -p pii-cli -p pii-mcp -p pii-web -p pii-evolve) \
    || abort "release build failed inside the clean export of $(git -C "$REPO" rev-parse --short HEAD)"
  BIN_SRC="$EXPORT/target/release"
  echo "BUILT FROM EXPORT: $(git -C "$REPO" rev-parse --short HEAD) (working-tree changes excluded)"
fi
for b in "${BINS[@]}" "${STAGED[@]}"; do
  [ -x "$BIN_SRC/$b" ] || abort "built binary $BIN_SRC/$b missing"
done

# ---- 2: move existing binaries aside (dated; never delete) ----
ASIDE="${PREFIX}.old-${TS}"
moved=0
for b in "${BINS[@]}"; do
  if [ -e "$PREFIX/$b" ]; then
    mkdir -p "$ASIDE"
    mv "$PREFIX/$b" "$ASIDE/$b"
    moved=$((moved + 1))
  fi
done

# ---- 3: install ----
for b in "${BINS[@]}"; do install -m 0755 "$BIN_SRC/$b" "$PREFIX/$b"; done
for b in "${STAGED[@]}"; do install -m 0755 "$BIN_SRC/$b" "$PREFIX/$b.new"; done

# ---- 4: runtime config from the production example ----
mkdir -p "$CONFIG_DIR" "$STATE_DIR/trajectory"
CONF="$CONFIG_DIR/pii.toml"
if [ -e "$CONF" ]; then
  mkdir -p "$ASIDE"
  mv "$CONF" "$ASIDE/pii.toml"
fi
TMPCONF="$(mktemp "$CONFIG_DIR/pii.toml.XXXXXX")"
sed \
  -e "s|^model = .*|model = \"llamacpp-server:granite-4.2-3b\"|" \
  -e "s|^runtime_dir = .*|runtime_dir = \"$RUNTIME_DIR\"|" \
  -e "s|^# trajectory_dir = .*|trajectory_dir = \"$STATE_DIR/trajectory\"|" \
  -e "s|^log_file = .*|log_file = \"$STATE_DIR/pii.log\"|" \
  -e "s|^host = .*|host = \"$LLAMA_HOST\"|" \
  "$REPO/examples/pii.production.toml" > "$TMPCONF"
cat >> "$TMPCONF" <<TOML

# --- appended by deploy/install-pii.sh (spec 142) ---
# ONE Discord forum channel. The token (PII_DISCORD_TOKEN) comes only from the unit's
# environment file, never from here. The forum id is written into the cron's input below
# (it is an id, not a secret).
[[channel]]
name = "discord"
kind = "discord"
graph = "discord-triage"

TOML
if [ "$HAVE_CHANNEL" = 1 ]; then
  cat >> "$TMPCONF" <<TOML

# ONE daily digest. \`schedule\` is the display mirror; firing is driven by the graph's timer node.
[[cron]]
name = "daily-digest"
graph = "discord-digest"
schedule = "0 9 * * *"
input = { to = "$PII_DISCORD_CHANNEL_ID" }
TOML
fi
mv "$TMPCONF" "$CONF"

# ---- 4a: the channel credentials, by reference to the operator's environment (spec 157) ----
# The file lives next to pii.toml, outside the repo, mode 0600, and is the only place the values go.
ENVF="$CONFIG_DIR/pii.env"
if [ "$HAVE_CHANNEL" = 1 ]; then
  if [ -e "$ENVF" ]; then
    mkdir -p "$ASIDE"
    mv "$ENVF" "$ASIDE/pii.env"
  fi
  TMPENV="$(mktemp "$CONFIG_DIR/pii.env.XXXXXX")"   # mktemp creates it 0600
  printf 'PII_DISCORD_TOKEN=%s\nPII_DISCORD_CHANNEL_ID=%s\n' "$PII_DISCORD_TOKEN" "$PII_DISCORD_CHANNEL_ID" > "$TMPENV"
  chmod 0600 "$TMPENV"
  mv "$TMPENV" "$ENVF"
fi

# ---- 4b: graphs into the persistent state dir (the unit places them in the runtime dir on each start) ----
mkdir -p "$STATE_DIR/graphs"
installed_graphs=()
for gf in "$REPO"/graphs/*.toml; do
  gn="$(basename "$gf")"
  dest="$STATE_DIR/graphs/$gn"
  if [ -e "$dest" ]; then
    cmp -s "$gf" "$dest" && { installed_graphs+=("$gn"); continue; }
    mkdir -p "$ASIDE"
    mv "$dest" "$ASIDE/graph.$gn"
  fi
  install -m 0644 "$gf" "$dest"
  installed_graphs+=("$gn")
done

# ---- 5: systemd unit (written, not enabled) ----
mkdir -p "$UNIT_DIR"
UNIT="$UNIT_DIR/pii.service"
if [ -e "$UNIT" ]; then
  mkdir -p "$ASIDE"
  mv "$UNIT" "$ASIDE/pii.service"
fi
if [ "$SCOPE" = system ]; then
  WANTED=multi-user.target; USERLINE="User=$(id -un)"
else
  WANTED=default.target; USERLINE=""
fi
cat > "$UNIT" <<UNITEOF
# Written by deploy/install-pii.sh (spec 142). Secrets live ONLY in $CONFIG_DIR/pii.env.
[Unit]
Description=Pii daemon
After=network-online.target

[Service]
Type=simple
$USERLINE
Environment=PII_RUNTIME_DIR=$RUNTIME_DIR
Environment=PII_CONFIG=$CONF
EnvironmentFile=-$CONFIG_DIR/pii.env
WorkingDirectory=$STATE_DIR
ExecStartPre=/usr/bin/env mkdir -p $RUNTIME_DIR
ExecStartPre=/usr/bin/env ln -sfn $STATE_DIR/graphs $RUNTIME_DIR/graphs
ExecStart=$PREFIX/pii daemon --foreground --config $CONF --store $STATE_DIR/store.redb --workspace $STATE_DIR --log-file $STATE_DIR/pii.log --log-rotate-bytes 10485760 --log-keep-files 5
Restart=always
RestartSec=2
TimeoutStopSec=10
NoNewPrivileges=yes

[Install]
WantedBy=$WANTED
UNITEOF
sed -i '/^$/d' "$UNIT"

# ---- 6: what the operator still has to do ----
echo "INSTALLED: $PREFIX/{${BINS[*]}} (pii-mcp staged as $PREFIX/pii-mcp.new, NOT swapped)"
echo "MOVED ASIDE: $moved binaries${ASIDE:+ -> $ASIDE}"
echo "GRAPHS: ${installed_graphs[*]} -> $STATE_DIR/graphs (placed in $RUNTIME_DIR/graphs on each unit start)"
echo "CONFIG: $CONF   UNIT: $UNIT"
echo
if [ "$HAVE_CHANNEL" = 1 ]; then
  echo "CHANNEL: $ENVF written (0600) from PII_DISCORD_TOKEN / PII_DISCORD_CHANNEL_ID; the first post names the revision"
else
  echo "CHANNEL: PII_DISCORD_TOKEN / PII_DISCORD_CHANNEL_ID not both set: no pii.env written, the daemon will report channels: []"
fi
echo
echo "OPERATOR COMMANDS (nothing below was run):"
echo "  1. channel credentials: rerun this script with both variables exported (values never printed), or place"
echo "     '$CONFIG_DIR/pii.env' yourself (mode 0600, never commit it)"
echo "  2. enable and start:"
echo "       $CTL daemon-reload && $CTL enable --now pii"
[ "$SCOPE" = user ] && echo "       loginctl enable-linger \"\$(id -un)\"   # keep it running after logout"
echo "  3. verify:  PII_RUNTIME_DIR='$RUNTIME_DIR' '$PREFIX/pii' health --json   (see _docs/05-operations/deploy-runbook.md)"
echo "  4. roll back:  bash deploy/rollback-pii.sh   (PREFIX='$PREFIX' SCOPE=$SCOPE)"
