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
#   ~/.local/state/pii/{*.redb,graphs}  state, migrated
#   ~/.config/systemd/user/pii.service  native unit, Restart=always, no container
#
# Verification: health must answer as the user, the unit must be active, and a kill must be followed by a restart.
set -uo pipefail
HOST=mvcorrea@lnx
BIN=/root/.local/bin/pii
SRC_CFG=/root/.config/pii/pii.toml
SRC_ENV=/root/.config/pii/pii.env
SRC_STATE=/root/.local/state/pii
say(){ printf '%s\n' "$*"; }
sshx(){ timeout 120 ssh -o BatchMode=yes "$HOST" "$@"; }

say "=== 0. stop everything that could hold the store, and clear the strays"
sshx 'systemctl --user stop pii.service 2>/dev/null; true'
sshx 'sudo -n systemctl stop pii.service 2>/dev/null; sudo -n systemctl disable pii.service 2>/dev/null; true'
sshx 'kill -TERM 1774097 2>/dev/null; sudo -n kill -TERM 642710 2>/dev/null; sleep 2; kill -9 1774097 2>/dev/null; sudo -n kill -9 642710 2>/dev/null; true'
sshx 'podman exec hermes pkill -TERM -f "pii daemon" 2>/dev/null; sleep 1; true'
say "strays cleared"

say "=== 1. the binary in ~/.local/bin"
scp -q "$BIN" "$HOST:/tmp/pii.user" || { say "scp failed"; exit 1; }
sshx 'install -m 0755 /tmp/pii.user ~/.local/bin/pii && rm -f /tmp/pii.user && ~/.local/bin/pii --version | head -2' 2>&1 | tail -2

say "=== 2. configuration + state under ~/.local"
# the container's paths become the user's paths; the runtime dir is the user's own
sed -e 's#/root/.local/state/pii#'"$HOME"'/.local/state/pii#g' \
    -e 's#/run/user/0/pii#/run/user/1000/pii#g' "$SRC_CFG" > /tmp/pii.user.toml
# (sed cannot know $HOME: rewrite the literal after the fact)
sed -i 's#/root/.local/state/pii#~/.local/state/pii#g' /tmp/pii.user.toml
sed -i "s#~/#$HOME/#g" /tmp/pii.user.toml
scp -q /tmp/pii.user.toml "$HOST:pii.user.toml"
scp -q "$SRC_ENV" "$HOST:pii.user.env"
sshx 'mkdir -p ~/.local/pii ~/.local/state/pii/graphs && mv ~/pii.user.toml ~/.local/pii/pii.toml && install -m 0600 ~/pii.user.env ~/.local/pii/pii.env && rm -f ~/pii.user.env && ls -l ~/.local/pii/' 2>&1 | tail -3
for f in store.redb memory.redb approval.redb heal.redb; do
  [ -f "$SRC_STATE/$f" ] && scp -q "$SRC_STATE/$f" "$HOST:state.$f" && sshx "install -m 0600 ~/state.$f ~/.local/state/pii/$f && rm -f ~/state.$f" 2>&1 | tail -1
done
tar -C "$SRC_STATE/graphs" -cf - . | sshx 'tar -C ~/.local/state/pii/graphs -xf -' 2>&1 | tail -1
say "state: $(sshx 'ls ~/.local/state/pii | tr "\n" " "')"

say "=== 3. the native unit (user scope, no container)"
sshx 'cat > ~/.config/systemd/user/pii.service' <<'UNIT'
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
