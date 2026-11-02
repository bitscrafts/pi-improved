#!/usr/bin/env bash
# pii.sh — the ONE operator tool for pii's production host. Replaces the pile of one-off scripts
# (deploy-332/333, which-channel, reset-daemons, closeout, verify-331 …) that each did one step once.
#
# The shape it enforces:
#   PRODUCTION  native per-user on lnx — ~/.local/bin/pii · config ~/.local/pii/pii.toml ·
#               state ~/.local/state/pii · systemd --user pii.service
#   DEVELOPMENT the daemon inside the `hermes` container, own store, and NO Discord channel: it must not poll
#               production's forum. Measured 2026-11-02: the dev daemon (old binary, 500 ms default) was reading
#               the same channel as production and exhausting the shared route bucket — production saw 429s even
#               after 34 s of silence, because the bucket was never ours alone.
#
# Usage: pii.sh <command> [args]
#   build          cargo build --release -p pii-cli (warm cache from scripts/autonomy/env.sh)
#   install        copy the binary to the host (old kept as pii.old-<epoch>) → restart → verify
#   verify         revision, unit state, digest, channel counters, poll failures
#   channel <id>   what a Discord channel id IS (read-only)
#   daemons        every pii daemon, host and container
#   logs           log size + rotation state, both sides
#   dev-silence    stop the DEV daemon polling production's channel, then restart it
set -uo pipefail

HOST=${PII_HOST:-mvcorrea@lnx}
REPO=$(cd "$(dirname "$0")/.." && pwd)
sshx(){ timeout "${SSH_TIMEOUT:-120}" ssh -o BatchMode=yes "$HOST" "$@"; }

bin(){
  local p
  for p in /projects/_cache/targets/release/pii "$REPO"/target/release/pii; do
    [ -x "$p" ] && { printf '%s\n' "$p"; return 0; }
  done
  return 1
}

cmd_build(){
  ( cd "$REPO" && . scripts/autonomy/env.sh && cargo build --release -p pii-cli ) || return 1
  local b; b=$(bin) || return 1
  echo "built: $(stat -c %s "$b") bytes, $(date -r "$b" -u +%H:%M:%SZ)"
}

cmd_verify(){
  sshx 'export PII_RUNTIME_DIR=/run/user/1000/pii
    echo "unit     : $(systemctl --user is-active pii.service) / $(systemctl --user is-enabled pii.service)"
    echo "health   : $(~/.local/bin/pii health 2>&1 | head -1)"
    echo "digest   : $(~/.local/bin/pii graph list 2>/dev/null | grep -i digest | head -1)"
    echo "since    : $(systemctl --user show pii.service -p ActiveEnterTimestamp --value)"
    echo "counters : $(grep -c "poll.failed" ~/.local/state/pii/pii.log 2>/dev/null || echo 0) poll failure(s), log $(stat -c %s ~/.local/state/pii/pii.log 2>/dev/null || echo 0) bytes"'
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

cmd_channel(){
  local id=${1:?usage: pii.sh channel <channel-id>}
  sshx "set -a; . \$HOME/.local/pii/pii.env; set +a
    printf 'header = \"Authorization: Bot %s\"\n' \"\$PII_DISCORD_TOKEN\" > /tmp/.pii-cfg; chmod 600 /tmp/.pii-cfg
    body=\$(curl -sS --config /tmp/.pii-cfg 'https://discord.com/api/v10/channels/$id' 2>/dev/null)
    rm -f /tmp/.pii-cfg
    printf '%s' \"\$body\" | tr ',' '\n' | grep -E '\"(name|type|guild_id)\"' | head -3"
}

cmd_daemons(){
  echo "-- host side:"
  sshx 'ps -eo pid,etime,rss,cmd | grep "[.]local/bin/pii daemon" | cut -c1-110'
  echo "-- container hermes:"
  ps -eo pid,etime,rss,cmd | grep "[p]ii daemon" | cut -c1-110 || echo "   none"
  echo "   (a container process also appears on the host with a different pid — that is one daemon, not two)"
}

cmd_logs(){
  echo "-- production:"
  sshx 'ls -l ~/.local/state/pii/pii.log* 2>/dev/null | awk "{print \$5, \$9}"; systemctl --user cat pii.service | grep -o -- "--log-rotate-bytes [0-9]*\|--log-keep-files [0-9]*" | sed "s/^/   /"'
  echo "-- loop:"
  printf '   files: %s\n' "$(find "$REPO/_tmp/autonomy/logs" -maxdepth 1 -name '*.log' 2>/dev/null | wc -l)"
}

cmd_dev_silence(){
  local cfg=/root/.config/pii/pii.toml
  [ -f "$cfg" ] || { echo "dev-silence: no $cfg" >&2; return 2; }
  cp -p "$cfg" "$cfg.before-dev-silence-$(date +%s)"
  # comment every non-comment line of the [[channel]] block, and nothing else
  awk '
    /^\[\[channel\]\]/ { inside=1; print "# dev: no production channel poll  " $0; n++; next }
    /^\[/ { inside=0 }
    inside && $0 !~ /^[[:space:]]*#/ && $0 !~ /^[[:space:]]*$/ { print "# dev: no production channel poll  " $0; n++; next }
    { print }
    END { if (n) printf("commented %d line(s)\n", n) > "/dev/stderr" }
  ' "$cfg" > "$cfg.new" && mv "$cfg.new" "$cfg" && chmod 600 "$cfg"
  pkill -f '/root/.local/bin/pii daemon' 2>/dev/null; sleep 1
  ( setsid nohup /root/.local/bin/pii daemon --foreground --config "$cfg" \
      --store /root/.local/state/pii/store.redb --workspace /root/.local/state/pii \
      --log-file /root/.local/state/pii/pii.log --log-rotate-bytes 10485760 --log-keep-files 5 \
      >/dev/null 2>&1 & )
  sleep 4
  echo "dev daemon processes: $(pgrep -fc '/root/.local/bin/pii daemon' || echo 0)"
  echo "dev channel blocks  : $(grep -c '^\[\[channel\]\]' "$cfg" || true)"
  echo "dev commented lines : $(grep -c '^# dev: no production channel poll' "$cfg" || true)"
}

# Empty a FORUM channel: every post is a thread, so emptying the channel means deleting threads. The work is done
# by deploy/tools/discord-forum-empty.py, run on the host so the token never leaves it. Without --yes it only lists.
cmd_channel_empty(){
  local forum=${1:?usage: pii.sh channel-empty <forum-id> [--yes]}
  local yes=""; [ "${2:-}" = "--yes" ] && yes="--yes"
  timeout 300 ssh -o BatchMode=yes "$HOST" "python3 - --forum $forum $yes" < "$REPO/deploy/tools/discord-forum-empty.py"
}

# The digest is ON DEMAND (standing decision, 2026-11-02): 1a/1b/1c are valuable when asked, not on a timer.
# This removes the `[[cron]]` block whose graph is discord-digest, cancels any suspended run of it, restarts once
# and shows that nothing is armed. The graph stays runnable by hand; only the schedule goes.
cmd_digest_off(){
  local tmp; tmp=$(mktemp -d /tmp/digest-off-XXXXXX)
  scp -q "$HOST:.local/pii/pii.toml" "$tmp/pii.toml" || { echo "digest-off: cannot fetch config" >&2; rm -rf "$tmp"; return 2; }
  python3 - "$tmp/pii.toml" <<'PY'
import re, sys
p = sys.argv[1]
text = open(p).read()
blocks = re.split(r'(?m)^(?=\[\[cron\]\])', text)
out, removed = [], 0
for b in blocks:
    if b.startswith('[[cron]]') and 'discord-digest' in b:
        removed += 1
        continue
    out.append(b)
open(p, 'w').write(''.join(out))
print(f"removed {removed} cron block(s) naming discord-digest")
PY
  timeout 60 ssh -o BatchMode=yes "$HOST" "cp -p ~/.local/pii/pii.toml ~/.local/pii/pii.toml.digestoff-\$(date +%s)" || { rm -rf "$tmp"; return 3; }
  scp -q "$tmp/pii.toml" "$HOST:.local/pii/pii.toml.new" && rm -rf "$tmp"
  timeout 120 ssh -o BatchMode=yes "$HOST" 'set -e
    install -m 600 ~/.local/pii/pii.toml.new ~/.local/pii/pii.toml && rm -f ~/.local/pii/pii.toml.new
    systemctl --user restart pii.service; sleep 3
    export PII_RUNTIME_DIR=/run/user/1000/pii
    for r in $(~/.local/bin/pii run ls --status suspended 2>/dev/null | awk "/r[0-9]/{print \$1}"); do
      ~/.local/bin/pii run cancel "$r" --reason "digest is on demand now" >/dev/null 2>&1 && echo "cancelled $r"
    done
    echo "unit   : $(systemctl --user is-active pii.service)"
    echo "cron   : $(grep -c "^\[\[cron\]\]" ~/.local/pii/pii.toml || true) block(s) in the config"
    echo "digest : $(~/.local/bin/pii graph list 2>/dev/null | grep -i digest || echo none)"
    echo "suspended: $(~/.local/bin/pii run ls --status suspended 2>/dev/null | wc -l) run(s)"'
}

case "${1:-}" in
  build)       cmd_build ;;
  install)     cmd_install ;;
  verify)      cmd_verify ;;
  channel)     shift; cmd_channel "$@" ;;
  daemons)     cmd_daemons ;;
  logs)        cmd_logs ;;
  dev-silence) cmd_dev_silence ;;
  channel-empty) shift; cmd_channel_empty "$@" ;;
  digest-off)  cmd_digest_off ;;
  *) sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'; exit 2 ;;
esac
