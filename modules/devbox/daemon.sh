#!/usr/bin/env bash
# modules/devbox/daemon.sh <up|status|down> — the pii daemon that implements the queue's specs, INSIDE the devbox container on the host.
# Idempotent: `up` starts the container (podman-compose) and the daemon only when they are not already answering.
#
# The daemon runs a COPY of the binary (_tmp/devbox-test/bin/pii), not target/debug/pii: a gate or a worker's own `cargo build` rewrites
# target/debug/pii from whatever WIP is in the tree, and the tool that implements a spec must not change under it. A new binary reaches the
# daemon only through `daemon.sh refresh` (build at the current tree, copy, restart) — a deliberate act, never a side effect.
set -uo pipefail
CMD=${1:-status}
HOST=${DEVBOX_HOST:-mvcorrea@10.10.10.217}
HP=${DEVBOX_PROJECT:-/podman/assistants/hermes/projects/pi-improved}
ssh -o BatchMode=yes -o ConnectTimeout=15 "$HOST" "CMD=$CMD HP=$HP bash -s" <<'REMOTE'
C=/projects/pi-improved/_tmp/devbox-test
CFG=$C/claude
PII=$C/bin/pii
X() { podman exec -w /projects/pi-improved pii-devbox "$@"; }
health() { X sh -c "env PII_RUNTIME_DIR=$C/run $PII health --json 2>/dev/null" | head -c 4000; }
up_container() {
  podman ps --filter name=pii-devbox --filter status=running --format '{{.Names}}' | grep -q pii-devbox && return 0
  ( cd "$HP/modules/devbox" && DEVBOX_PROJECT=$HP DEVBOX_ENV_FILE=$HOME/.config/pii/devbox.env podman-compose -f compose.yaml up -d 2>&1 | tail -2 )
}
start_daemon() {
  X sh -c "mkdir -p $C/bin $C/run $C/tmp $C/graphs $CFG && rm -f $C/run/control.sock && ln -sfn /run/claude-login/.credentials.json $CFG/.credentials.json"
  X sh -c "test -x $PII || bash /projects/pi-improved/modules/devbox/install-binary.sh \${CARGO_TARGET_DIR:-target} $PII" || return 1
  # PII_MODEL: the daemon refuses to start without one; a coder run never calls it. CLAUDE_CODE_TMPDIR: claude's scratch dir must be
  # somewhere the daemon's write sandbox allows (measured: EACCES on mkdir /tmp/claude-0). CLAUDE_CONFIG_DIR (spec 307): claude's conversation
  # file goes under the workspace, which the sandbox allows; /root/.claude is refused, so `--resume` could never find a session the daemon started.
  podman exec -d -w /projects/pi-improved pii-devbox sh -c "env PII_RUNTIME_DIR=$C/run PI_CODING_AGENT_DIR=/projects/pi-improved/_tmp/pii-agent IS_SANDBOX=1 PII_MODEL=llamacpp-server:granite-4.2-3b FRESH_GATE_TARGET=/projects/pi-improved/_tmp/gate-target CARGO_BUILD_JOBS=8 CLAUDE_CODE_TMPDIR=$C/tmp TMPDIR=$C/tmp CLAUDE_CONFIG_DIR=$CFG $PII daemon --foreground --store $C/store.redb --workspace /projects/pi-improved --log-file $C/pii.log > $C/daemon.out 2>&1"
  for i in $(seq 1 30); do [ -n "$(health)" ] && return 0; sleep 1; done
  echo "daemon did not answer in 30 s; daemon.out:"; X sh -c "head -c 400 $C/daemon.out"; return 1
}
case "$CMD" in
  up)      up_container; [ -n "$(health)" ] || start_daemon || exit 1; health | grep -o '"revision":"[^"]*"' | head -1 ;;
  status)  up_container; health | grep -o '"revision":"[^"]*"' | head -1 || echo "no daemon" ;;
  refresh) up_container
           X sh -c "mkdir -p $C/bin; pkill -x pii; sleep 1; cargo build -p pii-cli --bin pii 2>&1 | tail -2; bash /projects/pi-improved/modules/devbox/install-binary.sh \${CARGO_TARGET_DIR:-target} $PII" || exit 1
           start_daemon && health | grep -o '"revision":"[^"]*"' | head -1 && echo "daemon binary sha $(X sh -c "sha256sum $PII" | cut -c1-12)" ;;
  down)    X sh -c "pkill -x pii" ; echo "daemon stopped (container left up)" ;;
  *) echo "usage: daemon.sh up|status|refresh|down"; exit 2 ;;
esac
REMOTE
