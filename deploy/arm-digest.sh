#!/usr/bin/env bash
# arm-digest.sh — give the running daemon the channel credential it never received, so M5 can start counting.
#
# WHY THIS EXISTS (measured 2026-11-01). Production serves `669f5803d90a` and answers health with rc 0, and nothing
# has posted: `cron_entries: 0`, `armed: 0`, `channel_posts: 0`. The reason is not a bug in the daemon:
#   · `$CONFIG_DIR/pii.env` exists (mode 0600, both PII_DISCORD_* variables set), but the unit reads it through
#     `EnvironmentFile=`, which only applies to a SYSTEMD start -- and this container has no systemd bus, so the
#     daemon here is hand-started and its environment holds none of those variables;
#   · `install-pii.sh` appends the digest `[[cron]]` block only when HAVE_CHANNEL=1, and it was last run WITHOUT the
#     variables, so the live pii.toml has a `[[channel]]` block and no schedule.
# So the digest cannot arm: a channel with no credential and a config with no cron entry. This script fixes exactly
# those two, in that order, and proves each step before the next.
#
# It POSTS TO DISCORD once armed (the first digest at 12:00Z), so it runs only when a person asks: --apply is
# required, and the default is a plan that touches nothing.
#
# usage: arm-digest.sh [--apply] [--dry-run]
set -uo pipefail
REPO=${REPO:-/projects/pi-improved}
CONFIG_DIR=${CONFIG_DIR:-/root/.config/pii}   # CONFIG_DIR is overridable so --print-host can be driven by a test
ENVFILE="$CONFIG_DIR/pii.env"
APPLY=0
PRINT_HOST=0
for a in "$@"; do
  case "$a" in
    --apply) APPLY=1 ;;
    --dry-run) APPLY=0 ;;
    --print-host) PRINT_HOST=1 ;;
    *) echo "usage: arm-digest.sh [--apply]" >&2; exit 2 ;;
  esac
done
say(){ printf '%s\n' "$*"; }
die(){ say "ARM: REFUSED — $*"; exit 1; }

# 0) THE HOST MUST COME FROM THE [llama] SECTION, NOT FROM THE FIRST URL IN THE FILE. The first version of this script
# grepped the file for a URL and took the first match, which was a COMMENT ("# loopback, http://127.0.0.1:8080/v1");
# it passed LLAMA_HOST=127.0.0.1:8080 to the installer, whose merge wrote it into [llama] host -- and production's
# model is granite-4.2-3b on :8084, while :8080 serves laguna-xs-2.1. Every model call would have failed.
llama_host(){ awk '/^\[/{s=$0} s=="[llama]" && /^host *= */{gsub(/^host *= *"|".*$/,""); print; exit}' "$CONFIG_DIR/pii.toml"; }
if [ "$PRINT_HOST" = 1 ]; then llama_host; exit 0; fi

# 1) the credential must be there, and we must not print it
[ -f "$ENVFILE" ] || die "no $ENVFILE: there is nothing to arm with"
N=$(grep -cE '^PII_DISCORD_(TOKEN|CHANNEL_ID)=' "$ENVFILE" 2>/dev/null || echo 0)
[ "$N" -eq 2 ] || die "$ENVFILE holds $N of the 2 PII_DISCORD_* variables"
say "ARM: credential found (2 variables, $(stat -c '%s' "$ENVFILE") bytes, mode $(stat -c '%a' "$ENVFILE")) — values not shown"

# 2) what the daemon has today, before anything changes
PID=$(pgrep -f '/root/.local/bin/pii daemon' | head -1)
[ -n "${PID:-}" ] || die "no pii daemon found: start it, then arm the digest"
HAVE_ENV=$(tr '\0' '\n' < "/proc/$PID/environ" 2>/dev/null | grep -cE '^PII_DISCORD_'; true)
say "ARM: daemon pid $PID holds $HAVE_ENV PII_DISCORD_* variable(s) in its environment"
say "ARM: live config cron blocks: $(grep -c '^\[\[cron\]\]' "$CONFIG_DIR/pii.toml" 2>/dev/null; true)"

# 3) argv must come from /proc, never retyped (runbook 105: the hand-start argv is the only record of it)
ARGV=$(tr '\0' '\n' < "/proc/$PID/cmdline" | paste -sd' ' -)
[ -n "$ARGV" ] || die "could not read the daemon's argv from /proc/$PID/cmdline"
say "ARM: restart argv is the running one, read from /proc: $ARGV"

if [ "$APPLY" = 0 ]; then
  cat <<PLAN

PLAN (nothing has been changed; re-run with --apply):
  1. copy $CONFIG_DIR/pii.toml aside with a timestamp, and record its table-and-key inventory
  2. set -a; . $ENVFILE; set +a   (so install-pii.sh sees HAVE_CHANNEL=1)
  3. SKIP_BUILD=1 CONFIG_ONLY=1 LLAMA_HOST=<the live value> bash deploy/install-pii.sh
     -> appends the [[cron]] digest block that was never added
  4. verify the new config keeps EVERY table and key of the old one, or restore the old one and stop
  5. restart the daemon ONCE with that environment (argv from /proc), health must return 0 within 20 s
     or the previous binary and config are restored
  6. report health plus cron_entries / armed / channels, and the expected first post at 12:00Z
PLAN
  exit 0
fi

ASIDE="$CONFIG_DIR/pii.toml.arm-$(date -u +%Y%m%dT%H%M%SZ)"
cp -p "$CONFIG_DIR/pii.toml" "$ASIDE" || die "could not copy the live config aside"
say "ARM: config copied to $ASIDE"

# THE HOST MUST COME FROM THE [llama] SECTION, NOT FROM THE FIRST URL IN THE FILE. The first version of this script
# grepped the file for a URL and took the first match, which was a COMMENT ("# loopback, http://127.0.0.1:8080/v1");
# it then passed LLAMA_HOST=127.0.0.1:8080 to the installer, whose merge dutifully wrote it into [llama] host -- and
# production's model is granite-4.2-3b, which lives on :8084, while :8080 serves laguna-xs-2.1. Every model call
# would have failed. The value is read from the section, and verified by VALUE afterwards.
LLAMA_LIVE=$(llama_host)
[ -n "$LLAMA_LIVE" ] || die "could not read [llama] host from $CONFIG_DIR/pii.toml; refusing to guess"
say "ARM: [llama] host kept as $LLAMA_LIVE"

set -a
# shellcheck disable=SC1090
. "$ENVFILE"
set +a
say "ARM: environment loaded from $ENVFILE (HAVE_CHANNEL=${HAVE_CHANNEL:-unset})"

cd "$REPO" || die "no repo at $REPO"
if ! SKIP_BUILD=1 CONFIG_ONLY=1 LLAMA_HOST="$LLAMA_LIVE" bash deploy/install-pii.sh > /tmp/arm-digest-install.log 2>&1; then
  cp -p "$ASIDE" "$CONFIG_DIR/pii.toml"
  die "install-pii.sh failed (log /tmp/arm-digest-install.log); the old config was restored"
fi
CRON=$(grep -c '^\[\[cron\]\]' "$CONFIG_DIR/pii.toml"; true)
say "ARM: cron blocks in the new config: $CRON"
[ "$CRON" -ge 1 ] || die "the digest block was not appended: nothing was armed"

# 4) the digest may not cost the operator anything he wrote
if ! python3 deploy/pii-config-merge.py verify "$ASIDE" "$CONFIG_DIR/pii.toml" > /tmp/arm-digest-verify.log 2>&1; then
  cp -p "$ASIDE" "$CONFIG_DIR/pii.toml"
  die "the new config dropped something the old one had (see /tmp/arm-digest-verify.log); restored"
fi
# A key that exists with a different VALUE passes a key-level verify and still breaks production: that is exactly
# how the host was clobbered. These two values decide every model call, so they are compared as values.
for k in '^model *= ' '^host *= '; do
  A=$(grep -m1 "$k" "$ASIDE" | sed 's/#.*//; s/ *$//')
  B=$(grep -m1 "$k" "$CONFIG_DIR/pii.toml" | sed 's/#.*//; s/ *$//')
  if [ "$A" != "$B" ]; then
    cp -p "$ASIDE" "$CONFIG_DIR/pii.toml"
    die "the new config changed '$k' (was '$A', now '$B'); restored -- an armed digest must not cost the model"
  fi
  say "ARM: verified unchanged: $B"
done
say "ARM: every table, key and the two model values of the previous config survive"

# 5) one counted restart, with the environment, and rollback if health does not answer
kill -TERM "$PID" 2>/dev/null
for _ in $(seq 1 20); do kill -0 "$PID" 2>/dev/null || break; sleep 1; done
( set -a; . "$ENVFILE"; set +a; cd /projects && exec $ARGV ) >/tmp/arm-digest-daemon.log 2>&1 &
NEW=$!
for _ in $(seq 1 20); do
  if PII_RUNTIME_DIR=${PII_RUNTIME_DIR:-/run/user/0/pii} /root/.local/bin/pii health >/tmp/arm-digest-health.log 2>&1; then
    say "ARM: daemon restarted (pid $NEW) and health answers"
    say "ARM: $(tr '\0' '\n' < /proc/$NEW/environ | grep -cE '^PII_DISCORD_'; true) channel variable(s) now in its environment"
    say "ARM: the first digest is expected at 12:00Z; watch 'pii status --json' for channel_posts"
    exit 0
  fi
  sleep 1
done
say "ARM: FAILED — health did not answer within 20 s; the daemon log is /tmp/arm-digest-daemon.log"
say "ARM: the previous config is at $ASIDE"
exit 1
