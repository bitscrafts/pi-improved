#!/usr/bin/env bash
# The loop's environment, set here and not inherited.
#
# Measured 2026-10-11: a cycle started by the cron tick answered
#   UNDECIDED rc=127: timeout: failed to run command 'claude': No such file or directory
# because a scheduled job does not inherit the interactive PATH (claude lives in /root/.local/bin, cargo in
# /usr/local/cargo/bin). This is the same reason HARD RULE 0d exists: a dispatch that worked only because the
# calling shell happened to have the right PATH is not reproducible. Every script in this directory sources
# this file before it does anything.
# APPENDED, not prepended: a caller that injects its own PATH (the selftest's fake CLI, a scrubber, a
# wrapper) must keep priority, while a minimal scheduled environment still finds the tools it needs.
export PATH="${PATH:-}:/root/.local/bin:/usr/local/cargo/bin:/usr/local/bin:/usr/bin:/bin"
case ":$PATH:" in *":/root/.local/bin:"*) ;; *) export PATH="$PATH:/root/.local/bin" ;; esac
export PII_TMP_ROOT="${PII_TMP_ROOT:-/projects/_tmp}"

# root + bypassPermissions also needs the sandbox marker, or the worker dies in one second with
# "--dangerously-skip-permissions cannot be used with root/sudo privileges".
export IS_SANDBOX="${IS_SANDBOX:-1}"

# The gate's compiler cache. Measured 2026-10-15: cycle.sh exported CARGO_TARGET_DIR, but xtask/src/fresh_gate.rs
# reads FRESH_GATE_TARGET - so the export pinned its OWN empty target directory and every attempt paid a full cold
# workspace build, while _tmp/gate-target never even existed on disk. The gate's TREE stays fresh either way; only the
# compiler cache is reused. This is the name the gate actually reads.
export FRESH_GATE_TARGET="${FRESH_GATE_TARGET:-/projects/pi-improved/_tmp/gate-target}"

# BUILD PARALLELISM IS CAPPED. Measured 2026-10-17 04:52: load 81-88 on a 32-core machine with several workspace builds in
# flight (one worker, its clippy/check/test steps, and whatever else was building). Each one was free to claim all 32
# cores, so every model turn waited minutes on compiling and the account window stopped advancing for two hours while the
# console sat at 31%. Slower per build, far faster overall; the gate's own CMAKE setting is now bounded by the same idea.
: "${CARGO_BUILD_JOBS:=8}"
: "${CMAKE_BUILD_PARALLEL_LEVEL:=8}"
export CARGO_BUILD_JOBS CMAKE_BUILD_PARALLEL_LEVEL
