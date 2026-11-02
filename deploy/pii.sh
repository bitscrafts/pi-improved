#!/usr/bin/env bash
# pii.sh — the ONE operator tool for pii's production host. Replaces the pile of one-off scripts
# (deploy-332/deploy-333/which-channel/reset-daemons/closeout/verify-331 …) that each did one step once.
#
# The shape it enforces: production is a NATIVE per-user install on lnx —
#   ~/.local/bin/pii · config ~/.local/pii/pii.toml · state ~/.local/state/pii · systemd --user pii.service
# development is the daemon inside the `hermes` container, with its own store and no scheduled posts.
#
# Usage: pii.sh <command> [args]
#   build              cargo build --release -p pii-cli (warm cache from scripts/autonomy/env.sh)
#   install            build → copy to the host (old binary kept as pii.old-<epoch>) → restart → health
#   verify             revision, unit state, digest armed, channel counters, recent errors
#   channel <id>       what a Discord channel id actually IS (name, type, guild) — read-only
#   daemons            every pii daemon on the host and in the container, counted precisely
#   logs               log size + rotation state on both sides
set -uo pipefail

HOST=${PII_HOST:-mvcorrea@lnx}
USER_DIR=/home/mvcorrea
REPO=$(cd "$(dirname "$0")/.." && pwd)
sshx(){ timeout "${SSH_TIMEOUT:-120}" ssh -o BatchMode=yes "$HOST" "$@"; }

cmd_build(){
  ( cd "$REPO" && . scripts/autonomy/env.sh && cargo build --release -p pii-cli ) || return 1
  ls -l "$(bin)" | awk '{print "built:", $5, "bytes", $6, $7, $8}'
}

bin(){
  for p in /projects/_cache/targets/release/pii "$REPO"/target/release/pii; do
    [ -x "$p" ] && { echo "$p"; return 0; }
  done
  return 1
}

cmd_install(){
  local b; b=$(bin) || { echo "install: no release binary — run 'pii.sh build'" >&2; return 2; }
  scp -q "$b" "$HOST:/tmp/pii.new" || { echo "install: copy failed" >&2; return 3; }
  sshx 'set -e
    cp -p ~/.local/bin/pii ~/.local/bin/pii.old-$(date +%s)
    systemctl --user stop pii.service
    install -m 0755 /tmp/pii.new ~/.local/bin/pii && rm -f /tmp/pii.new
    systemctl --user start pii.service
    sleep 3' || { echo "install: restart failed" >&2; return 4; }
  cmd_verify
}

cmd_verify(){
  sshx 'export PII_RUNTIME_DIR=/run/user/1000/pii
    echo "unit     : $(systemctl --user is-active pii.service) / $(systemctl --user is-enabled pii.service)"
    echo "health   : $(~/.local/bin/pii health 2>&1 | head -1)"
    echo "digest   : $(~/.local/bin/pii graph list 2>/dev/null | grep -i digest | head -1)"
    echo "since    : $(systemctl --user show pii.service -p ActiveEnterTimestamp --value)"
    echo "errors   : $(grep -c "\"level\":\"error\"" ~/.local/state/pii/pii.log 2>/dev/null || echo 0) error line(s), $(grep -c "poll.failed" ~/.local/state/pii/pii.log 2>/dev/null || echo 0) poll failure(s)"'
}

cmd_channel(){
  local id=${1:?usage: pii.sh channel <channel-id>}
  sshx "set -a; . \$HOME/.local/pii/pii.env; set +a
    printf 'header = \"Authorization: Bot %s\"\n' \"\$PII_DISCORD_TOKEN\" > /tmp/.pii-cfg; chmod 600 /tmp/.pii-cfg
    curl -sS --config /tmp/.pii-cfg 'https://discord.com/api/v10/channels/$id' > /tmp/.pii-chan.json 2>/dev/null
    rm -f /tmp/.pii-cfg
    python3 -c \"import json;d=json.load(open('/tmp/.pii-chan.json'));print('name:',d.get('name'),'| type:',d.get('type'),'(0 text, 15 forum)','| guild:',d.get('guild_id'))\" 2>/dev/null || echo 'could not read that channel'"
}

cmd_daemons(){
  echo "-- host:"
  sshx 'ps -eo pid,etime,rss,cmd | grep "[.]local/bin/pii daemon" | cut -c1-120'
  echo "-- container hermes:"
  ( ps -eo pid,etime,rss,cmd | grep "[p]ii daemon" | cut -c1-120 ) || echo "   none"
}

cmd_logs(){
  echo "-- production:"
  sshx 'ls -l ~/.local/state/pii/pii.log* 2>/dev/null | awk "{print \$5, \$9}"; echo "   rotation: $(grep -o -- "--log-rotate-bytes [0-9]*" <(systemctl --user cat pii.service) | head -1) $(grep -o -- "--log-keep-files [0-9]*" <(systemctl --user cat pii.service) | head -1)"'
  echo "-- loop (repo):"
  ( cd "$REPO" && ls _tmp/autonomy/logs/*.log 2>/dev/null | wc -l | sed 's/^/   files: /' )
}

case "${1:-}" in
  build)   cmd_build ;;
  install) cmd_install ;;
  verify)  cmd_verify ;;
  channel) shift; cmd_channel "$@" ;;
  daemons) cmd_daemons ;;
  logs)    cmd_logs ;;
  *) sed -n '2,17p' "$0" | sed 's/^# \{0,1\}//'; exit 2 ;;
esac
