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
#   CONFIG_ONLY=1 rewrite only the runtime config and channel credentials (spec 329): no build, no deploy card,
#                 no binaries, no unit, no llama probe. The config is promoted exactly as in a full install.
#   PII_PYTHON    Python 3.11+ for tomllib     (default: python3 on PATH, else uv into _tmp/; see config-python.sh)
# Graphs: exactly the set named in graphs/minimal.txt (spec 384), plus the two the generated config names; each lints, and the set is closed.
# Preconditions (checked before anything is moved): graphs/discord-{triage,digest}.toml exist, llama host answers, PREFIX is creatable.
# PII_DISCORD_TOKEN / PII_DISCORD_CHANNEL_ID are optional: without them no pii.env is written and the daemon
# starts with `channels: []` (spec 157).
set -euo pipefail

# PATH, set inside the script: measured 2026-10-20, the release build inside the clean export died with
# "cargo: command not found" because a non-interactive shell does not carry the caller's PATH. A deploy script that only
# works when the invoker happens to have cargo on PATH is not re-runnable (HARD RULE 0d).
export PATH="/root/.local/bin:/usr/local/cargo/bin:/usr/local/bin:/usr/bin:/bin:/usr/local/sbin:/usr/sbin:/sbin:${PATH:-}"
export CARGO_HOME="${CARGO_HOME:-/usr/local/cargo}"
export RUSTUP_HOME="${RUSTUP_HOME:-/usr/local/rustup}"

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
DEPLOY_CARD_DIR="${DEPLOY_CARD_DIR:-$REPO/_tmp/deploy}"   # spec 197: overridable for the check only
BINS=(pii pii-web pii-evolve)   # swapped in place
STAGED=(pii-mcp)                # installed as <name>.new only: the pii-mcp swap is a separate step (runbook)
TS="$(date +%s)"
ASIDE="${PREFIX}.old-${TS}"
. "$REPO/deploy/config-python.sh"
PY="$(config_python "$REPO")" || { echo "ABORT: no Python 3.11+ (tomllib); set PII_PYTHON" >&2; exit 2; }   # spec 329: reads and verifies the config
MERGE="$REPO/deploy/pii-config-merge.py"

abort() { echo "ABORT (nothing was changed): $*" >&2; exit 1; }

# ---- 4 + 4a: the runtime config and the channel credentials (spec 329) ----
# A re-install starts from the operator's file, not the template: the replaced file is the base, the template supplies
# only what that base lacks, and the installer edits only the keys it owns. The promotion is refused unless every table
# and key path of the replaced file is still in the candidate; the replaced file is then restored (R4).
fail_restore() {
  if [ -n "${PRIOR:-}" ]; then cp -p "$PRIOR" "$CONF"; fi
  rm -f "$TMPCONF" "$TMPCONF.channel" "$TMPCONF.cron"
  echo "ABORT: $*" >&2
  exit 1
}
write_config() {
  mkdir -p "$CONFIG_DIR" "$STATE_DIR/trajectory"
  CONF="$CONFIG_DIR/pii.toml"
  PRIOR=""
  if [ -e "$CONF" ]; then
    # Checked before the file moves: an unparseable config is refused with the prior file byte-identical.
    "$PY" -I "$MERGE" check "$CONF" || abort "the config to be replaced does not parse as TOML (named above)"
    mkdir -p "$ASIDE"
    mv "$CONF" "$ASIDE/pii.toml"
    PRIOR="$ASIDE/pii.toml"
  fi
  TMPCONF="$(mktemp "$CONFIG_DIR/pii.toml.XXXXXX")"
  cat > "$TMPCONF.channel" <<TOML

# --- appended by deploy/install-pii.sh (spec 142) ---
# ONE Discord forum channel. The token (PII_DISCORD_TOKEN) comes only from the unit's
# environment file, never from here. The forum id is written into the cron's input below
# (it is an id, not a secret).
[[channel]]
name = "discord"
kind = "discord"
graph = "discord-triage"
TOML
  MERGE_ARGS=(--template "$REPO/examples/pii.production.toml" --out "$TMPCONF" --block "$TMPCONF.channel"
    --set "core.runtime_dir=$RUNTIME_DIR" --set "core.trajectory_dir=$STATE_DIR/trajectory"
    --set "core.log_file=$STATE_DIR/pii.log" --set "llama.host=$LLAMA_HOST")
  if [ -n "$PRIOR" ]; then MERGE_ARGS+=(--prior "$PRIOR"); fi
  if [ "$HAVE_CHANNEL" = 1 ]; then
    cat > "$TMPCONF.cron" <<TOML

# ONE daily digest at 12:00Z = 09:00 Brazil (UTC-3). `schedule` is the display mirror; firing is
# driven by the graph's timer node, which carries the same 12:00Z knob (measured 2026-10-19: the
# mirror and the timer node must say the same hour, and the name must match what a replaced
# install already recorded — a re-install regresses the operator truth otherwise).
[[cron]]
name = "digest"
graph = "discord-digest"
schedule = "0 12 * * *"
input = { to = "$PII_DISCORD_CHANNEL_ID" }
TOML
    MERGE_ARGS+=(--block "$TMPCONF.cron")
  fi
  "$PY" -I "$MERGE" merge "${MERGE_ARGS[@]}" || fail_restore "the config could not be built (named above)"
  # R4: every table and key path of the replaced file must survive; a loss restores the replaced file.
  if [ -n "$PRIOR" ] && ! "$PY" -I "$MERGE" verify "$PRIOR" "$TMPCONF"; then
    fail_restore "the promoted config lost keys the replaced config had (named above); refused"
  fi
  mv "$TMPCONF" "$CONF"
  rm -f "$TMPCONF.channel" "$TMPCONF.cron"

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
}

# ---- R3: preconditions, before anything is built or moved ----
HAVE_CHANNEL=0
[ -n "${PII_DISCORD_TOKEN:-}" ] && [ -n "${PII_DISCORD_CHANNEL_ID:-}" ] && HAVE_CHANNEL=1

# ---- R5: the config half alone: no build, no card, no binaries, no unit, no network precondition ----
if [ "${CONFIG_ONLY:-0}" = 1 ]; then
  write_config
  echo "CONFIG ONLY: $CONF (no build, no deploy card, no binaries, no unit, no llama probe)"
  exit 0
fi

# Spec 197 (070 step 5): no install without the deploy card for the revision about to be built.
# Cards are keyed by the 7-char short revision; the binary reports 12. Either keying passes.
REV7="$(git -C "$REPO" rev-parse --short=7 HEAD)" || abort "cannot read HEAD of $REPO — is it a git checkout?"
REV12="$(git -C "$REPO" rev-parse --short=12 HEAD)"
deploy_card_check "$DEPLOY_CARD_DIR" "$REV7" "$REV12" \
  || abort "no deploy card for revision $REV7: expected $DEPLOY_CARD_DIR/card-$REV7.json or $DEPLOY_CARD_DIR/card-$REV12.json"
command -v curl >/dev/null || abort "curl not found"
curl -sf --max-time 5 "${LLAMA_HOST%/}/models" >/dev/null \
  || abort "llama.cpp host ${LLAMA_HOST} does not answer GET ${LLAMA_HOST%/}/models"
if [ ! -d "$PREFIX" ]; then
  mkdir -p "$PREFIX" 2>/dev/null || abort "target prefix $PREFIX does not exist and cannot be created"
fi
[ -w "$PREFIX" ] || abort "target prefix $PREFIX is not writable"

# Spec 384 / derivation 047: a user graph pins a system graph by sha256. When an upgrade replaces a
# system graph, the OLD bytes are retained under <graphdir>/.by-sha256/<digest>.toml, so the pin still
# resolves and the user graph keeps running exactly what it reviewed. Re-resolving the pin with a
# report would be worse: it silently changes what runs. `pii graph repin` is how a person moves it.
retain_replaced_graph(){ # retain_replaced_graph <installed-file> <destination-dir>
  local f="$1" dir="$2"
  [ -f "$f" ] || return 0
  local d; d=$(sha256sum "$f" | cut -d' ' -f1)
  mkdir -p "$dir/.by-sha256" 2>/dev/null || return 0
  [ -f "$dir/.by-sha256/$d.toml" ] || cp -p "$f" "$dir/.by-sha256/$d.toml" 2>/dev/null || true
}

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

# ---- R2-R4 (spec 384): the manifest decides what ships. Every named graph lints, the set is closed and every pin matches,
# all before anything moves. The graphs the generated config names ship too, because the daemon cannot start a channel without them.
MANIFEST="$REPO/graphs/minimal.txt"
[ -f "$MANIFEST" ] || abort "the graph manifest $MANIFEST is missing"
mapfile -t SHIP < <(sed -e 's/#.*//' -e 's/[[:space:]]//g' "$MANIFEST" | grep -v '^$' || true)
SHIP+=("${GRAPH_NAMES[@]}")
[ "${#SHIP[@]}" -gt 0 ] || abort "the graph manifest $MANIFEST names no graphs"
"$PY" -I "$REPO/deploy/graph-closure.py" "$REPO/graphs" "${SHIP[@]}" \
  || abort "the shipped graph set is not closed or not pinned (named above)"
LINT_DIR="$(mktemp -d "${TMPDIR:-/tmp}/pii-lint-XXXXXX")" || abort "cannot create a lint scratch directory"
for g in "${SHIP[@]}"; do
  (cd "$LINT_DIR" && PII_RUNTIME_DIR="$LINT_DIR" "$BIN_SRC/pii" graph lint "$REPO/graphs/$g.toml" >/dev/null 2>&1) \
    || abort "graph '$g' does not lint and is not shipped: run $BIN_SRC/pii graph lint $REPO/graphs/$g.toml"
done

# ---- 2: move existing binaries aside (dated; never delete) ----
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

# ---- 4 + 4a: the runtime config and the channel credentials ----
write_config

# ---- 4b: graphs into the persistent state dir (the unit places them in the runtime dir on each start) ----
mkdir -p "$STATE_DIR/graphs"
# A graph that was shipped before and is not in this set moves aside, so the directory names exactly the set.
for old in "$STATE_DIR"/graphs/*.toml; do
  [ -e "$old" ] || continue
  if ! printf '%s\n' "${SHIP[@]}" | grep -qx "$(basename "$old" .toml)"; then
    mkdir -p "$ASIDE"
    mv "$old" "$ASIDE/graph.$(basename "$old")"
  fi
done
installed_graphs=()
for g in "${SHIP[@]}"; do
  gf="$REPO/graphs/$g.toml"
  gn="$g.toml"
  dest="$STATE_DIR/graphs/$gn"
  if [ -e "$dest" ]; then
    cmp -s "$gf" "$dest" && { installed_graphs+=("$gn"); continue; }
    mkdir -p "$ASIDE"
    retain_replaced_graph "$dest" "$STATE_DIR/graphs"   # an old pin still resolves (spec 384)
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
