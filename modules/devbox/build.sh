#!/usr/bin/env bash
# modules/devbox/build.sh — build the devbox image ON THE HOST (the only machine with podman). Re-runnable; reads, copies and builds,
# deletes nothing. Run from the orchestrator: it stages the build context over ssh, then builds there.
#
# Context = Containerfile + the two agent binaries the orchestrator itself runs (pi, claude), staged to a directory on the host's
# root filesystem (NOT /podman, which is 95% full). Nothing under /root/.pi is read: only the `pi` binary and the claude ELF.
#
# Prints, before it spends anything: host, tag, the context payload size and the pi/claude versions it will bake in.
set -euo pipefail
export PATH="/root/.local/bin:/usr/local/cargo/bin:$PATH"
HOST=${DEVBOX_HOST:-mvcorrea@10.10.10.217}
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
REV=$(git -C "$REPO" rev-parse --short HEAD)
TAG=${DEVBOX_TAG:-localhost/pii-devbox:$REV}
CTX=${DEVBOX_CTX:-devbox-ctx}            # relative to the host user's home
PI=$(command -v pi); CL=$(readlink -f "$(command -v claude)")
[ -x "$PI" ] && [ -x "$CL" ] || { echo "pi or claude not found on PATH"; exit 2; }
echo "host      : $HOST"
echo "tag       : $TAG   (and :latest)"
echo "pi        : $($PI --version 2>&1 | head -1)   $(du -h "$PI" | cut -f1)"
echo "claude    : $($CL --version 2>&1 | head -1)   $(du -h "$CL" | cut -f1)"
echo "context   : ~/$CTX on the host"
ssh -o BatchMode=yes "$HOST" "mkdir -p ~/$CTX"
scp -q -o BatchMode=yes "$HERE/Containerfile" "$HOST:$CTX/Containerfile"
# rsync is not assumed on the host; scp to a temp name then mv, so a partial copy never replaces a good one.
scp -q -o BatchMode=yes "$PI" "$HOST:$CTX/pi.new"
scp -q -o BatchMode=yes "$CL" "$HOST:$CTX/claude.new"
ssh -o BatchMode=yes "$HOST" "cd ~/$CTX && mv -f pi.new pi && mv -f claude.new claude && ls -l Containerfile pi claude | cut -c1-90"
echo "== build (no cache flags: a cached layer from an earlier build is fine, the smoke step at the end is not cached away)"
ssh -o BatchMode=yes "$HOST" "cd ~/$CTX && podman build --pull=missing -t '$TAG' -t localhost/pii-devbox:latest -f Containerfile . 2>&1" | tail -${DEVBOX_TAIL:-40}
echo "== result"
ssh -o BatchMode=yes "$HOST" "podman image inspect localhost/pii-devbox:latest --format 'id={{.Id}} size={{.Size}} created={{.Created}}'; podman image inspect docker.io/library/rust:1.99.1-slim-trixie --format 'base digest={{.Digest}}'"
