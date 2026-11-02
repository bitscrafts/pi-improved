#!/usr/bin/env bash
# final-verify.sh — clean the residue of my failed attempts and prove the final topology:
#   production: native on the host, per-user, config under ~/.local, systemd --user, digest armed
#   development: in the hermes container, digest disarmed
#   nothing else: no /etc/pii, no /var/lib/pii, no system unit, no image, no quadlet
set -uo pipefail
HOST=mvcorrea@lnx
say(){ printf '%s\n' "$*"; }
sshx(){ timeout 90 ssh -o BatchMode=yes "$HOST" "$@"; }

say "=== 1. remove what my failed attempts left on the host"
sshx 'sudo -n rm -f /etc/systemd/system/pii.service; sudo -n systemctl daemon-reload
      sudo -n rm -rf /etc/pii /var/lib/pii /run/pii
      rm -f ~/.config/containers/systemd/pii.container
      podman rmi localhost/pii:latest 2>&1 | tail -1
      echo "system unit: $(ls /etc/systemd/system/pii.service 2>/dev/null || echo gone)"
      echo "/etc/pii: $(ls -d /etc/pii 2>/dev/null || echo gone) · /var/lib/pii: $(ls -d /var/lib/pii 2>/dev/null || echo gone)"
      echo "quadlet: $(ls ~/.config/containers/systemd/pii.container 2>/dev/null || echo gone) · image: $(podman images --format "{{.Repository}}:{{.Tag}}" | grep -c "^localhost/pii:")"' 2>&1 | tail -6

say "=== 2. PRODUCTION (host, native, per-user)"
sshx 'echo "unit: $(systemctl --user is-enabled pii.service) / $(systemctl --user is-active pii.service) pid=$(systemctl --user show pii.service -p MainPID --value)"
      echo "binary: $(ls -l ~/.local/bin/pii | awk "{print \$5\" bytes\"}") $(~/.local/bin/pii --version)"
      echo "config: $(ls -l ~/.local/pii/pii.toml | awk "{print \$5\" bytes\"}") · env: $(ls -l ~/.local/pii/pii.env | awk "{print \$1}")"
      echo "state: $(ls ~/.local/state/pii | tr "\n" " ")"
      PII_RUNTIME_DIR=/run/user/1000/pii ~/.local/bin/pii health 2>&1 | head -2
      echo "digest: $(PII_RUNTIME_DIR=/run/user/1000/pii ~/.local/bin/pii graph list 2>/dev/null | grep discord-digest)"' 2>&1 | tail -6

say "=== 3. DEVELOPMENT (hermes container)"
say "processes: $(pgrep -fc '/root/.local/bin/pii daemon' || echo 0)"
PII_RUNTIME_DIR=/run/user/0/pii /root/.local/bin/pii health 2>&1 | head -2
say "digest in the dev config: $(grep -c '^\[\[cron\]\]' /root/.config/pii/pii.toml 2>/dev/null; true) active block(s) (0 = disarmed)"

say "=== 4. the whole machine, counted precisely"
say "host production processes: $(sshx 'ps -eo cmd | grep -c "[/]home/mvcorrea/.local/bin/pii daemon"')"
say "container dev processes:    $(ps -eo cmd | grep -c "[/]root/.local/bin/pii daemon")"
