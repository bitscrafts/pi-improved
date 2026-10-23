#!/usr/bin/env bash
# modules/devbox/pii-build.sh — build the pii binary INSIDE the devbox (Rust 1.99.1, the same toolchain as the gate), into the warm
# build volume. Prints the binary's path, size, and the git revision it was built from. Re-runnable; incremental.
set -uo pipefail
HOST=${DEVBOX_HOST:-mvcorrea@10.10.10.217}
ssh -o BatchMode=yes "$HOST" 'podman exec -w /projects/pi-improved pii-devbox sh -c "
  cargo build -p pii-cli --bin pii 2>&1 | tail -3
  ls -l /projects/pi-improved/target/debug/pii | cut -c1-80
  echo built-from: \$(git rev-parse --short HEAD) dirty=\$(git status --porcelain | grep -v -c \"QUEUE-BRIEF\|__pycache__\")
  /projects/pi-improved/target/debug/pii --version 2>&1 | head -1"'
