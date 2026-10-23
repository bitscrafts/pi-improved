#!/usr/bin/env bash
# modules/devbox/gate.sh — run the quality gate INSIDE the devbox container, on the host, with the project bind-mounted at /projects/pi-improved.
# Re-runnable. Starts the devbox through podman-compose (creating nothing but an EMPTY 0600 env file if none exists: no key is
# copied or read), then `podman exec`s the gate. The container's writable layer is not relied on; build output lives in the
# named volume devbox-target.
#
# Prints the verdict lines and the container's own measurements, and the exit code of the gate. A gate that disagrees with the
# orchestrator-side gate is a finding about THIS container first (HARD RULE 0b: suspect the test/harness before production).
set -uo pipefail
HOST=${DEVBOX_HOST:-mvcorrea@10.10.10.217}
HP=${DEVBOX_PROJECT:-/podman/assistants/hermes/projects/pi-improved}   # the same tree this orchestrator sees as /projects/pi-improved
STEP=${DEVBOX_GATE_CMD:-bash quality-gate.sh}
ENVF='$HOME/.config/pii/devbox.env'
ssh -o BatchMode=yes "$HOST" "bash -s" <<REMOTE
set -u
umask 077
[ -e $ENVF ] || { : > $ENVF; echo "created EMPTY env file $ENVF (0600): add provider keys there yourself; none were copied"; }
stat -c '%a %U %n' $ENVF
cd $HP/modules/devbox || exit 2
export DEVBOX_PROJECT=$HP DEVBOX_ENV_FILE=$ENVF
podman-compose -f compose.yaml up -d 2>&1 | tail -4
echo "== inside the container"
podman exec pii-devbox bash -c 'echo "user=\$(id -un) cwd=\$(pwd) target=\$CARGO_TARGET_DIR"; rustc --version; ls | head -3; touch .devbox-owner-probe && stat -c "owner of a file written from inside: uid=%u" .devbox-owner-probe; rm -f .devbox-owner-probe'
echo "== gate: $STEP"
podman exec -w /projects/pi-improved pii-devbox bash -c '$STEP' > ~/devbox-gate.log 2>&1
echo "gate exit code: \$?"
REMOTE
echo "== verdict lines (log: ~/devbox-gate.log on the host)"
ssh -o BatchMode=yes "$HOST" "grep -a -E '^(PASS|FAIL)' ~/devbox-gate.log | grep -v 'unreadable\|store down'; tail -3 ~/devbox-gate.log | cut -c1-160"
