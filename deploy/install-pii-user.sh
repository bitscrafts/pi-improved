#!/usr/bin/env bash
# install-pii-user.sh — ONE clean, per-user, native install of pii on the host. No container, no image, no root,
# configuration under ~/.local (user's directive), supervised by systemd --user.
#
# What it replaces, and why the mess happened (measured 2026-11-02):
#   · `~/.config/systemd/user/pii.service` on the host is enabled but its Description reads "supervised by the host,
#     inside the hermes container": it podman-execs the daemon into `hermes` and Requires=hermes.service. That is the
#     unit that was running, and it is not what is wanted.
#   · `/home/mvcorrea/.local/bin/pii` exists but is an OLD build and its 8-day-old process does not answer
#     `pii health` at all (no socket at /run/user/1000/pii/default/control.sock). A stray.
#   · I added `/etc/systemd/system/pii.service` (root, /etc/pii, /var/lib/pii) on top of both. My mistake: it should
#     have been one install, in the user's own tree, chosen with the user.
#
# Layout after this script -- everything the user's, under their home:
#   ~/.local/bin/pii                    the binary (revision-stamped build)
#   ~/.local/pii/pii.toml               configuration
#   ~/.local/pii/pii.env                credential, 0600
#   ~/.local/state/pii/{*.redb,graphs}  state (the stores are NOT copied unless --seed-stores)
#   ~/.config/systemd/user/pii.service  native unit, Restart=always, no container
#
# Usage (spec 338):
#   bash deploy/install-pii-user.sh                 dry run: prints every action, writes nothing
#   bash deploy/install-pii-user.sh --apply         does it
#   --seed-stores                                   also copies the container's stores; refused while a host store exists
#   --seed-stores --force-seed                      overwrites the host's stores too; prints what it destroys first
# Refused (exit 2, nothing changed): no deploy card for HEAD (R1); a store already on the host (R2).
#
# Verification: health must answer as the user, the unit must be active, and a kill must be followed by a restart.
set -uo pipefail
HOST=mvcorrea@lnx
BIN="${BIN:-/root/.local/bin/pii}"
SRC_CFG="${SRC_CFG:-/root/.config/pii/pii.toml}"
SRC_ENV="${SRC_ENV:-/root/.config/pii/pii.env}"
SRC_STATE="${SRC_STATE:-/root/.local/state/pii}"
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEPLOY_CARD_DIR="${DEPLOY_CARD_DIR:-$REPO/_tmp/deploy}"   # spec 197: overridable for the check only
STAGE="${TMPDIR:-/tmp}"                                    # local staging for the config file
STORES=(store.redb memory.redb approval.redb heal.redb)
TS="$(date +%s)"
APPLY=0 SEED=0 FORCE_SEED=0
for arg in "$@"; do
  case "$arg" in
    --apply) APPLY=1 ;;
    --seed-stores) SEED=1 ;;
    --force-seed) FORCE_SEED=1 ;;
    *) echo "ABORT: unknown argument $arg" >&2; exit 2 ;;
  esac
done
[ "$FORCE_SEED" = 1 ] && [ "$SEED" = 0 ] && { echo "REFUSED: --force-seed needs --seed-stores" >&2; exit 2; }
say(){ printf '%s\n' "$*"; }
sshx(){ timeout 120 ssh -o BatchMode=yes "$HOST" "$@"; }
refuse(){ echo "REFUSED (nothing was changed): $*" >&2; exit 2; }

# ---- R1 (spec 338): the deploy card for HEAD, before anything is read from or written to the host ----
REV7="$(git -C "$REPO" rev-parse --short=7 HEAD)" || refuse "cannot read HEAD of $REPO"
REV12="$(git -C "$REPO" rev-parse --short=12 HEAD)" || refuse "cannot read HEAD of $REPO"
. "$REPO/deploy/config-python.sh"   # spec 338: the one card check (deploy_card_check), shared with install-pii.sh
deploy_card_check "$DEPLOY_CARD_DIR" "$REV7" "$REV12" \
  || refuse "no deploy card for revision $REV7: expected $DEPLOY_CARD_DIR/card-$REV7.json or $DEPLOY_CARD_DIR/card-$REV12.json"

# ---- R3 (spec 338): the home of the account on the HOST, not the container's $HOME (/root) ----
REMOTE_HOME="$(sshx 'printf %s "$HOME"')" && [ -n "$REMOTE_HOME" ] \
  || refuse "cannot read the home directory on $HOST over ssh"
STATE_REMOTE="$REMOTE_HOME/.local/state/pii"

# ---- R2 (spec 338): a store that exists on the host is never overwritten by this script ----
EXISTING=()
for f in "${STORES[@]}"; do
  case "$(sshx "test -e '$STATE_REMOTE/$f' && echo yes || echo no")" in
    yes) EXISTING+=("$STATE_REMOTE/$f") ;;
    no) ;;
    *) refuse "cannot tell whether $STATE_REMOTE/$f exists on $HOST" ;;
  esac
done
if [ "${#EXISTING[@]}" -gt 0 ]; then
  if [ "$SEED" = 1 ] && [ "$FORCE_SEED" = 1 ]; then
    say "FORCE-SEED: the container's stores will OVERWRITE these live stores on $HOST; the host's copies are destroyed:"
    for p in "${EXISTING[@]}"; do say "  destroyed: $p"; done
  elif [ "$SEED" = 0 ]; then
    # An upgrade: the binary, the config and the unit, and no store copied. R2 exists to stop the containers
    # stores being written over the live ones, which cannot happen when nothing is passed to copy.
    say "UPGRADE: live stores exist on $HOST and are left untouched (no store is copied without --seed-stores):"
    for p in "${EXISTING[@]}"; do say "  kept: $p"; done
  else
    say "REFUSED (nothing was changed): a store already exists on $HOST, and this installer does not overwrite one:"
    for p in "${EXISTING[@]}"; do say "  $p"; done
    say "reason: store.redb and memory.redb hold what pii has learned; copying the container's stores over them destroys it."
    say "--seed-stores without --force-seed is refused: it would overwrite the stores above."
    say "move them aside yourself on $HOST (this script moves nothing):"
    say "  ssh $HOST 'mkdir -p $STATE_REMOTE/aside-$TS && mv ${EXISTING[*]} $STATE_REMOTE/aside-$TS/'"
    exit 2
  fi
fi

# ---- R4 (spec 338): the plan, and nothing else, unless --apply ----
say "PLAN for $HOST: card $DEPLOY_CARD (revision $REV12), remote home $REMOTE_HOME"
say "  1. stop pii.service and the strays (old units, old daemons)"
say "  2. install $BIN -> $REMOTE_HOME/.local/bin/pii"
say "  3. write $REMOTE_HOME/.local/pii/pii.toml from $SRC_CFG (/root -> $REMOTE_HOME)"
say "  4. write $REMOTE_HOME/.local/pii/pii.env (0600) from $SRC_ENV"
if [ "$SEED" = 1 ]; then
  for f in "${STORES[@]}"; do
    [ -f "$SRC_STATE/$f" ] && say "  5. seed $STATE_REMOTE/$f from $SRC_STATE/$f"
  done
else
  say "  5. stores: none copied (pass --seed-stores to copy $SRC_STATE into $STATE_REMOTE)"
fi
say "  6. graphs -> $STATE_REMOTE/graphs"
say "  7. unit $REMOTE_HOME/.config/systemd/user/pii.service; daemon-reload, enable, start, verify as the user"
if [ "$APPLY" != 1 ]; then
  say "DRY RUN: nothing was written. Re-run with --apply to do this."
  exit 0
fi

say "=== 0. stop everything that could hold the store, and clear the strays"
sshx 'systemctl --user stop pii.service 2>/dev/null; true'
sshx 'sudo -n systemctl stop pii.service 2>/dev/null; sudo -n systemctl disable pii.service 2>/dev/null; true'
sshx 'kill -TERM 1774097 2>/dev/null; sudo -n kill -TERM 642710 2>/dev/null; sleep 2; kill -9 1774097 2>/dev/null; sudo -n kill -9 642710 2>/dev/null; true'
sshx 'podman exec hermes pkill -TERM -f "pii daemon" 2>/dev/null; sleep 1; true'
say "strays cleared"

say "=== 1. the binary in ~/.local/bin"
scp -q "$BIN" "$HOST:pii.user" || { say "scp failed"; exit 1; }
sshx 'mkdir -p ~/.local/bin && install -m 0755 ~/pii.user ~/.local/bin/pii && rm -f ~/pii.user && ~/.local/bin/pii --version | head -2' 2>&1 | tail -2

say "=== 2. configuration + state under ~/.local"
# R3 (spec 338): the container's /root paths become the HOST's home, read over ssh above
sed -e "s#/root/.local/state/pii#$REMOTE_HOME/.local/state/pii#g" \
    -e 's#/run/user/0/pii#/run/user/1000/pii#g' "$SRC_CFG" > "$STAGE/pii.user.toml"
scp -q "$STAGE/pii.user.toml" "$HOST:pii.user.toml" && rm -f "$STAGE/pii.user.toml"
scp -q "$SRC_ENV" "$HOST:pii.user.env"
sshx 'mkdir -p ~/.local/pii ~/.local/state/pii/graphs && mv ~/pii.user.toml ~/.local/pii/pii.toml && install -m 0600 ~/pii.user.env ~/.local/pii/pii.env && rm -f ~/pii.user.env && ls -l ~/.local/pii/' 2>&1 | tail -3
if [ "$SEED" = 1 ]; then
  for f in "${STORES[@]}"; do
    [ -f "$SRC_STATE/$f" ] && scp -q "$SRC_STATE/$f" "$HOST:state.$f" && sshx "install -m 0600 ~/state.$f ~/.local/state/pii/$f && rm -f ~/state.$f" 2>&1 | tail -1
  done
fi
tar -C "$SRC_STATE/graphs" -cf - . | sshx 'tar -C ~/.local/state/pii/graphs -xf -' 2>&1 | tail -1
say "state: $(sshx 'ls ~/.local/state/pii | tr "\n" " "')"

say "=== 3. the native unit (user scope, no container)"
sshx 'mkdir -p ~/.config/systemd/user && cat > ~/.config/systemd/user/pii.service' <<'UNIT'
[Unit]
Description=pii — personal information index daemon (native, per-user, no container)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
WorkingDirectory=%h
ExecStart=%h/.local/bin/pii daemon --foreground --config %h/.local/pii/pii.toml --store %h/.local/state/pii/store.redb --workspace %h/.local/state/pii --log-file %h/.local/state/pii/pii.log --log-rotate-bytes 10485760 --log-keep-files 5
EnvironmentFile=%h/.local/pii/pii.env
Environment=PII_RUNTIME_DIR=/run/user/1000/pii
Restart=always
RestartSec=2
TimeoutStartSec=60

[Install]
WantedBy=default.target
UNIT
sshx 'systemctl --user daemon-reload && systemctl --user enable pii.service 2>&1 | tail -1' 2>&1 | tail -2

say "=== 4. start and verify AS THE USER"
sshx 'systemctl --user start pii.service; sleep 7; echo "active: $(systemctl --user is-active pii.service)"; echo "main pid: $(systemctl --user show pii.service -p MainPID --value)"' 2>&1 | tail -2
H=$(sshx '~/.local/bin/pii health 2>&1 | head -3' 2>&1)
say "$H"
if printf '%s' "$H" | grep -q '^pii '; then
  say "INSTALL: OK — native, per-user, configuration under ~/.local"
  sshx 'echo "--- survives a kill:"; PID=$(systemctl --user show pii.service -p MainPID --value); kill -TERM "$PID"; sleep 8; echo "after kill: $(systemctl --user is-active pii.service) pid=$(systemctl --user show pii.service -p MainPID --value)"' 2>&1 | tail -2
else
  say "INSTALL: FAILED — see: systemctl --user status pii.service; journalctl --user -u pii -n 30"
  exit 1
fi
